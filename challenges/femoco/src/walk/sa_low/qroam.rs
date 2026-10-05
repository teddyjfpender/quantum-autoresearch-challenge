//! A clean-QROAM read whose junk is measured at once but whose phase fixup is deferred and merged
//! with the output's own erasure: one fixup over the whole index per read instead of two.
//!
//! `shared::lookup::qroam_load` measures the `lambda - 1` junk blocks right after the swap network
//! and pays a fixup over the whole index (`E(L)`) there; erasing the output later pays another
//! `E(L)`. The phases `(-1)^(m_junk . junk(x))` and `(-1)^(m_out . data(x))` multiply, so a single
//! fixup with the concatenated outcomes and data cancels both (Berry et al. 2019, App. C: fixups
//! compose). The pending phase depends only on the index, which is unchanged between the read and
//! the erasure (both inside one inner copy, before its `Reflect`), so deferring it is exact.
//!
//! Toffolis: read `ceil(L / lambda) + (lambda - 1) w`; erasure `E(L) = 2 (2^h - 1) + ceil(L / 2^h)`.
use super::range::{iterate_range, range_cost};
use crate::circuit::{Bit, Builder, Op, OperationType, Qubit};
use crate::walk::shared::lookup::{
    measure, phase_fixup, set_bits, swap_network, swap_perm, word, xor, xor_lookup, Data, Word,
};
use crate::walk::shared::unary::iterate;

/// A read in flight: the output register, the junk outcomes and the block layout.
pub struct Read {
    pub out: Vec<Qubit>,
    junk: Vec<Bit>,
    a: usize,
    w: usize,
    limit: u64,
    full: u64,
    /// First index value read (0 except for [`load_range`]).
    start: u64,
    /// Lever `X` ([`load_excl`]): junk block `l` holds `data(hv, l) ^ data(hv, 0)` instead of the
    /// swap network's permuted block.
    excl: bool,
}

/// A read with no junk (one block) whose output `out` already holds `data(x)` for `start <= x <
/// limit` and 0 elsewhere: the paired unary lookup (`paired.rs`, lever `G`)
/// hands its output to the walk's erasures through this.
#[must_use]
pub fn read_from_parts(out: Vec<Qubit>, w: usize, limit: u64, start: u64) -> Read {
    Read {
        out,
        junk: Vec::new(),
        a: 0,
        w,
        limit,
        full: limit,
        start,
        excl: false,
    }
}

impl Read {
    /// What junk position `pos` (1-based block) held, as a function of the index value `x`, as
    /// set bits of a `w`-bit word over `start..limit`.
    fn junk_word(&self, x: u64, data: &dyn Fn(u64) -> Word, pos: usize, perm: &[usize]) -> Word {
        let lam = 1u64 << self.a;
        let hv = x / lam;
        let get = |y: u64| -> Word {
            if (self.start..self.limit).contains(&y) {
                data(y)
            } else {
                word(self.w)
            }
        };
        if self.excl {
            let base = get(hv * lam);
            xor(&get(hv * lam + pos as u64), &base)
        } else {
            get(hv * lam + perm[pos] as u64)
        }
    }
}

/// Loads `data(x)` for the lane's `index` value `x < limit` into a fresh `w`-qubit register with
/// `2^a` blocks over the low index bits; the junk blocks are measured (freed) at once.
pub fn load(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    w: usize,
    data: &dyn Fn(u64) -> Word,
    a: usize,
) -> Read {
    let a = a.min(index.len());
    let (low, high) = index.split_at(a);
    let lam = 1u64 << a;
    let hi_limit = limit.div_ceil(lam);
    let full = (hi_limit * lam).min(1u64 << index.len().min(63));
    let blocks: Vec<Vec<Qubit>> = (0..lam).map(|_| b.alloc_n(w)).collect();
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
        xor_lookup(b, high, hi_limit, &flat, &wide);
    }
    swap_network(b, low, &blocks, false);
    let mut it = blocks.into_iter();
    let out = it.next().expect("at least one block");
    let rest: Vec<Qubit> = it.flatten().collect();
    let junk = measure(b, rest);
    Read {
        out,
        junk,
        a,
        w,
        limit,
        full,
        start: 0,
        excl: false,
    }
}

/// Lever `X`: [`load`] with the swap network replaced by the exclusive
/// multiplexer (`excl::mux`). The iteration over the high bits loads `data(hv, 0)` (plus the
/// multiplexer's classical left-over terms) into the output and `D_l = data(hv, l) ^ data(hv, 0)`
/// into `lambda - 1` blocks; the low bits are written one-hot in place; `out ^= sum_l x_l D_l`
/// costs `(lambda - 1) w / 2` Toffolis (pairs, and a bit-pair-shared triple for odd counts)
/// instead of `(lambda - 1) w`; the one-hot is erased by measurement (Clifford). The blocks hold
/// functions of the high index only and are measured at once (junk, as before).
pub fn load_excl(
    b: &mut Builder,
    index: &[Qubit],
    limit: u64,
    w: usize,
    data: &dyn Fn(u64) -> Word,
    a: usize,
) -> Read {
    let a = a.min(index.len());
    if a == 0 {
        return load(b, index, limit, w, data, 0);
    }
    let (low, high) = index.split_at(a);
    let lam = 1u64 << a;
    let hi_limit = limit.div_ceil(lam);
    let full = (hi_limit * lam).min(1u64 << index.len().min(63));
    let out = b.alloc_n(w);
    let mut regs: Vec<Vec<Qubit>> = vec![Vec::new()];
    for _ in 1..lam {
        regs.push(b.alloc_n(w));
    }
    let get = |y: u64| -> Word {
        if y < limit {
            data(y)
        } else {
            word(w)
        }
    };
    let wide = |hv: u64| -> Word {
        let base = get(hv * lam);
        let mut ds: Vec<Word> = vec![word(w)];
        for l in 1..lam {
            ds.push(xor(&get(hv * lam + l), &base));
        }
        let first = xor(&base, &super::excl::mux_leftover(&ds, w));
        let mut outw = word(lam as usize * w);
        for bit in set_bits(&first, w) {
            outw[bit / 64] |= 1 << (bit % 64);
        }
        for (l, d) in ds.iter().enumerate().skip(1) {
            for bit in set_bits(d, w) {
                let at = l * w + bit;
                outw[at / 64] |= 1 << (at % 64);
            }
        }
        outw
    };
    let flat: Vec<Qubit> = out
        .iter()
        .chain(regs[1..].iter().flatten())
        .copied()
        .collect();
    if high.is_empty() {
        for bit in set_bits(&wide(0), flat.len()) {
            b.x(flat[bit]);
        }
    } else {
        xor_lookup(b, high, hi_limit, &flat, &wide);
    }
    let hot = super::excl::low_hot(b, low);
    super::excl::mux(b, &hot.x, &regs, &out);
    super::excl::low_unhot(b, low, hot);
    let rest: Vec<Qubit> = regs[1..].iter().flatten().copied().collect();
    let junk = measure(b, rest);
    Read {
        out,
        junk,
        a,
        w,
        limit,
        full,
        start: 0,
        excl: true,
    }
}

/// [`load`] over the index values `start..limit` only (`start` a multiple of `2^a`): the
/// iteration over the high bits skips everything below `start >> a` (`range::iterate_range`),
/// and the output reads 0 outside the range.
pub fn load_range(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &dyn Fn(u64) -> Word,
    a: usize,
) -> Read {
    let a = a.min(index.len());
    let (low, high) = index.split_at(a);
    let lam = 1u64 << a;
    assert!(
        start.is_multiple_of(lam),
        "range start must be block aligned"
    );
    assert!(!high.is_empty(), "a range read needs high index bits");
    let hi_limit = limit.div_ceil(lam);
    let full = (hi_limit * lam).min(1u64 << index.len().min(63));
    let blocks: Vec<Vec<Qubit>> = (0..lam).map(|_| b.alloc_n(w)).collect();
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
        for bit in set_bits(&wide(hv), flat.len()) {
            b.cx(flag, flat[bit]);
        }
    };
    iterate_range(b, None, high, start / lam, hi_limit, &mut leaf);
    swap_network(b, low, &blocks, false);
    let mut it = blocks.into_iter();
    let out = it.next().expect("at least one block");
    let rest: Vec<Qubit> = it.flatten().collect();
    let junk = measure(b, rest);
    Read {
        out,
        junk,
        a,
        w,
        limit,
        full,
        start,
        excl: false,
    }
}

impl Read {
    /// The junk outcomes, for a later read of the same table with the same layout to cancel in
    /// its own fixup ([`erase_with`]); the output qubits are the caller's to return to `|0>`.
    #[must_use]
    pub fn into_junk(self) -> Vec<Bit> {
        self.junk
    }
}

/// Erases `r.out` (holding `data(x)`) and cancels both its phase and the junk's with one fixup
/// over `index` split at `h` low bits.
pub fn erase(b: &mut Builder, index: &[Qubit], r: Read, data: &dyn Fn(u64) -> Word, h: usize) {
    erase_with(b, index, r, Vec::new(), data, h);
}

/// [`erase`], also cancelling the junk phase `extra` of an earlier read of the same table at the
/// same index with the same layout (its junk outcomes, [`Read::into_junk`]): the two junk phases
/// have the same data, so the fixup reads that data twice. Same Toffolis as [`erase`].
///
/// # Panics
/// If `extra` is neither empty nor as long as `r`'s junk.
pub fn erase_with(
    b: &mut Builder,
    index: &[Qubit],
    r: Read,
    extra: Vec<Bit>,
    data: &dyn Fn(u64) -> Word,
    h: usize,
) {
    let out = measure(b, r.out.clone());
    erase_all(b, index, r, extra, out, data, h);
}

/// [`erase`] for a read whose output the caller has already measured, bit for bit, into `out`
/// (in the output's bit order): one fixup over `index` for the junk and those outcomes
/// (`sa-pareto`).
pub fn erase_measured(
    b: &mut Builder,
    index: &[Qubit],
    r: Read,
    out: Vec<Bit>,
    data: &dyn Fn(u64) -> Word,
    h: usize,
) {
    erase_all(b, index, r, Vec::new(), out, data, h);
}

/// One fixup for `r`'s junk, `extra` (an earlier read's junk, or nothing) and the output's
/// outcomes `out`.
fn erase_all(
    b: &mut Builder,
    index: &[Qubit],
    r: Read,
    extra: Vec<Bit>,
    out: Vec<Bit>,
    data: &dyn Fn(u64) -> Word,
    h: usize,
) {
    assert_eq!(out.len(), r.w, "one outcome per output bit");
    let mut r = r;
    let junk = std::mem::take(&mut r.junk);
    let lam = 1u64 << r.a;
    let perms: Vec<Vec<usize>> = (0..lam).map(|l| swap_perm(r.a, l)).collect();
    let (w, limit) = (r.w, r.limit);
    let junk_w = (lam as usize - 1) * w;
    assert!(
        extra.is_empty() || extra.len() == junk_w,
        "extra junk layout"
    );
    let copies = if extra.is_empty() { 1 } else { 2 };
    let total = copies * junk_w + w;
    let both = |x: u64| -> Word {
        let l = x % lam;
        let mut out = word(total);
        for pos in 1..lam as usize {
            let jw = r.junk_word(x, data, pos, &perms[l as usize]);
            for bit in set_bits(&jw, w) {
                for c in 0..copies {
                    let at = c * junk_w + (pos - 1) * w + bit;
                    out[at / 64] |= 1 << (at % 64);
                }
            }
        }
        if x < limit {
            for bit in set_bits(&data(x), w) {
                let at = copies * junk_w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
        out
    };
    let mut bits = junk;
    bits.extend(extra);
    bits.extend(out);
    phase_fixup(b, index, r.full, &bits, &both, h);
}

/// `phase_fixup` where the control `ctl` is 1: applies `(-1)^(m . data(x))` on the lanes whose
/// `index` holds `x < limit` and whose `ctl` is 1. The one-hot of the low `h` index bits is
/// written under the control (so it is empty where `ctl` is 0), which costs 2 Toffolis more
/// than the uncontrolled fixup: `2 (2^h - 1) + 2 + ceil(limit / 2^h)` (`sa-pareto`).
pub fn phase_fixup_ctl(
    b: &mut Builder,
    ctl: Qubit,
    index: &[Qubit],
    limit: u64,
    m: &[Bit],
    data: &dyn Fn(u64) -> Word,
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
        let mut leaf = |b: &mut Builder, v: u64, flag: Qubit| b.cx(flag, hot[v as usize]);
        iterate(b, Some(ctl), low, hot.len() as u64, &mut leaf);
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

/// How a phase fixup splits its index: `h` low bits written one-hot, and whether that one-hot
/// register is erased by measurement plus a second fixup over the low bits split at `hot`
/// (`Some(h2)`), instead of by a second one-hot pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub h: usize,
    pub hot: Option<usize>,
}

/// Toffolis of `iterate(None, bits, limit)` (the unary-iteration tree without a control).
#[must_use]
pub fn iterate_cost(bits: usize, limit: u64) -> u64 {
    fn node(ctl: bool, bits: usize, base: u64, limit: u64) -> u64 {
        if bits == 0 {
            return 0;
        }
        let half = 1u64 << (bits - 1);
        let right = base + half < limit;
        let own = u64::from(ctl);
        own + node(true, bits - 1, base, limit)
            + if right {
                node(true, bits - 1, base + half, limit)
            } else {
                0
            }
    }
    if bits == 0 {
        return 0;
    }
    node(false, bits, 0, limit.min(1u64 << bits.min(63)))
}

/// Toffolis of [`fixup`] over `bits` index bits and `limit` values.
#[must_use]
pub fn fixup_cost(bits: usize, limit: u64, s: Split) -> u64 {
    fixup_cost_range(bits, 0, limit, s)
}

/// Toffolis of [`fixup_range`].
#[must_use]
pub fn fixup_cost_range(bits: usize, start: u64, limit: u64, s: Split) -> u64 {
    let h = s.h.min(bits);
    let hot = 1u64 << h;
    let one_hot = if h > 0 { iterate_cost(h, hot) } else { 0 };
    let high = if bits > h {
        range_cost(bits - h, start >> h, limit.div_ceil(hot))
    } else {
        0
    };
    let undo = match s.hot {
        Some(h2) if h > 0 => fixup_cost(h, hot, Split { h: h2, hot: None }),
        _ => one_hot,
    };
    one_hot + high + undo
}

/// The cheapest split for a fixup over `bits` index bits and `limit` values; with `hot` the
/// one-hot register may be erased by measurement.
#[must_use]
pub fn best_split(bits: usize, limit: u64, hot: bool) -> Split {
    best_split_range(bits, 0, limit, hot)
}

/// [`best_split`] for the values `start..limit`.
#[must_use]
pub fn best_split_range(bits: usize, start: u64, limit: u64, hot: bool) -> Split {
    let mut best = Split { h: 0, hot: None };
    let mut cost = fixup_cost_range(bits, start, limit, best);
    for h in 0..=bits.min(12) {
        let opts: Vec<Option<usize>> = if hot && h > 0 {
            std::iter::once(None).chain((0..=h).map(Some)).collect()
        } else {
            vec![None]
        };
        for o in opts {
            let s = Split { h, hot: o };
            let c = fixup_cost_range(bits, start, limit, s);
            if c < cost {
                cost = c;
                best = s;
            }
        }
    }
    best
}

/// `shared::lookup::phase_fixup`, with the option ([`Split::hot`]) of erasing the one-hot
/// register by X-basis measurement: an outcome vector `m_hot` leaves the phase
/// `(-1)^(m_hot[v])` on the lanes whose low bits hold `v`, a table over the `h` low bits alone,
/// which a second, small fixup cancels (Berry et al. 2019, App. C, applied to the one-hot
/// register itself). Costs [`fixup_cost`].
pub fn fixup(b: &mut Builder, index: &[Qubit], limit: u64, m: &[Bit], data: &Data<'_>, s: Split) {
    if s.hot.is_none() {
        phase_fixup(b, index, limit, m, data, s.h);
        return;
    }
    fixup_range(b, index, 0, limit, m, data, s);
}

/// [`fixup`] on the lanes whose index lies in `start..limit` (the others are untouched).
pub fn fixup_range(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    m: &[Bit],
    data: &Data<'_>,
    s: Split,
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
    let h = s.h.min(index.len());
    assert!(h > 0 || s.hot.is_none(), "a measured one-hot needs h > 0");
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
            if x < start {
                continue;
            }
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
        iterate_range(b, None, high, start >> h, limit.div_ceil(lam), leaf);
    }
    let Some(h2) = s.hot else {
        one_hot(b);
        hot.into_iter().for_each(|q| b.free(q));
        return;
    };
    // hot[v] = [low = v] for every v < 2^h: measuring leaves (-1)^(m_hot[low]).
    let mh = measure(b, hot);
    let unit = |v: u64| -> Word {
        let mut w = word(mh.len());
        w[(v / 64) as usize] |= 1 << (v % 64);
        w
    };
    phase_fixup(b, low, lam, &mh, &unit, h2);
}

/// Erases a read whose output register the caller has already measured (`out_bits`, in slot
/// order), cancelling in one [`fixup`] the junk's phase (`orig`, the table the read loaded),
/// the output's (`content(x)`: what each slot held when it was measured, as a function of the
/// index) and that of `extra` bits measured elsewhere (`extra_data(x)`).
#[allow(clippy::too_many_arguments)]
pub fn erase_parts(
    b: &mut Builder,
    index: &[Qubit],
    r: Read,
    orig: &Data<'_>,
    out_bits: Vec<Bit>,
    content: &Data<'_>,
    extra: Vec<Bit>,
    extra_data: &Data<'_>,
    s: Split,
) {
    assert_eq!(out_bits.len(), r.w, "one outcome per output slot");
    let mut r = r;
    let junk = std::mem::take(&mut r.junk);
    let lam = 1u64 << r.a;
    let perms: Vec<Vec<usize>> = (0..lam).map(|l| swap_perm(r.a, l)).collect();
    let (w, limit) = (r.w, r.limit);
    let junk_w = (lam as usize - 1) * w;
    let ex = extra.len();
    let total = junk_w + w + ex;
    let start = r.start;
    let all = |x: u64| -> Word {
        let l = x % lam;
        let mut out = word(total);
        for pos in 1..lam as usize {
            let jw = r.junk_word(x, orig, pos, &perms[l as usize]);
            for bit in set_bits(&jw, w) {
                let at = (pos - 1) * w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
        if (start..limit).contains(&x) {
            for bit in set_bits(&content(x), w) {
                let at = junk_w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
            for bit in set_bits(&extra_data(x), ex) {
                let at = junk_w + w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
        out
    };
    let mut bits = junk;
    bits.extend(out_bits);
    bits.extend(extra);
    if start == 0 {
        fixup(b, index, r.full, &bits, &all, s);
    } else {
        fixup_range(b, index, start, r.full, &bits, &all, s);
    }
}

impl Read {
    /// Output width.
    #[must_use]
    pub fn width(&self) -> usize {
        self.w
    }
}

/// [`erase`] with a [`Split`] (a measured one-hot when `s.hot` is set; `sa-toff`).
pub fn erase_split_by(b: &mut Builder, index: &[Qubit], r: Read, data: &Data<'_>, s: Split) {
    let mut r = r;
    let junk = std::mem::take(&mut r.junk);
    let rout = std::mem::take(&mut r.out);
    let lam = 1u64 << r.a;
    let perms: Vec<Vec<usize>> = (0..lam).map(|l| swap_perm(r.a, l)).collect();
    let (w, limit) = (r.w, r.limit);
    let junk_w = (lam as usize - 1) * w;
    let both = |x: u64| -> Word {
        let l = x % lam;
        let mut out = word(junk_w + w);
        for pos in 1..lam as usize {
            let jw = r.junk_word(x, data, pos, &perms[l as usize]);
            for bit in set_bits(&jw, w) {
                let at = (pos - 1) * w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
        if x < limit {
            for bit in set_bits(&data(x), w) {
                let at = junk_w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
        out
    };
    assert_eq!(r.start, 0, "erase_split_by reads from 0");
    let mut bits = junk;
    bits.extend(measure(b, rout));
    fixup(b, index, r.full, &bits, &both, s);
}
