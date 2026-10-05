//! Lever `-`: the inner read from a **folded, `i_0`-refined item one-hot**.
//!
//! The paired item one-hot (`itemhot.rs`, levers `H V`) reads the `(item, i)` table at
//! `w L / (2 S)` Toffolis per copy (`L` cells, `S = e1` slots, two groups): one paired Toffoli per
//! word bit per inner index. Refining the one-hot by the inner index's low bit `i_0` halves the
//! iteration (32 leaves over `i' = i >> 1` instead of 64) but quadruples the virtual groups
//! `(g, i_0)` over the same `e1` slots, so on its own it saves nothing (`(4 / 2) x 32 = 64` per bit,
//! as before). This lever **folds** one of the four virtual groups, `(g, i_0) = (1, 1)`,
//! into `B = ceil(e1 / 3)` fresh slots `nu`, so the lane sees **three** exclusive groups over
//! `e1 + B` slots: `1.5` Toffolis per bit per leaf with lever `Z`'s shared triple, `48` per bit on Li
//! instead of `64`, for `B` more qubits during the read only.
//!
//! **The fold.** Slot `s` of the item one-hot lies in block `k = s div B` (`k < 3`). With
//! `d = g AND i_0` (one AND, erased by measurement at once), `nu_t = d AND P_t(h)` with
//! `P_t = h_t ^ h_(t + B) ^ h_(t + 2B)` (`B` ANDs; parities in place). On a folded lane
//! (`d = 1`, in range) **both** `h_s` and `nu_(s mod B)` are 1: the old slot is not cleared (that
//! would cost a Toffoli per slot). The lane's group is
//!
//! - `(g, i_0) = (0, 0)`: group 0; `(1, 0)`: group 1; `(0, 1)`: group 2 (unfolded lanes);
//! - `(1, 1)`: group `k`, the block of its slot (folded lanes),
//!
//! and the group bits are made in place: `g <- g ^ c ^ c B1(h)`, `i_0 <- i_0 ^ c ^ c B2(h)` with
//! `c = parity(nu)` (1 exactly on the folded lanes) and `B_k` the parity of block `k`'s slots
//! (two Toffolis). Out-of-range lanes (empty one-hot) keep `(g, i_0)` and may have both bits set; the
//! gated lines are made with group 1 first (`u1 = f AND g'`, `f ^= u1`, `u2 = f AND i_0'`,
//! `f ^= u2`, `u0 = f`), so they are exclusive on every lane.
//!
//! **Coefficients.** Group `j`'s word at leaf `i'` is a vector over the `e1 + B` slots. On unfolded
//! lanes only `h_s` is hot, so its `h` part is the plain table: group 0 `W(0, s, 2i')`, group 1
//! `W(1, s, 2i')`, group 2 `W(0, s, 2i' + 1)` (`W(g, s, i)` the word of the item at slot `s`, group
//! `g`). A folded lane in block `k` sees `h_s ^ nu_t` (`t = s - k B`), so `nu_t`'s coefficient in
//! group `k` is `W(1, s, 2i' + 1) ^ (group k's h coefficient at s)`; the sum is the folded item's
//! word. Every product is lever `z`'s paired form `(alpha.u ^ X)(beta.u ^ Y)` with exclusive lines,
//! so linear coefficients add.
//!
//! **Left-overs.** A product leaves `X Y` on every lane. On a one-hot that is linear (CNOT fan-out
//! from the slots, as in `itemhot`); on a folded lane's two hot bits it is
//! `x_s y_s ^ x_t y_t ^ (x_s y_t ^ x_t y_s)`: the first two terms are the same per-slot fan-out (both
//! slots fire), the cross term is a function of `s` on the folded lanes only, so it is removed at the
//! end by one Toffoli per output bit, `(c, Q_k(h))` (`c = parity(nu)`, `Q_k` the accumulated cross
//! vector; parities in place). Phases (lever `t`'s in-pass sign) take `CZ` in place of the Toffoli
//! target, `Z` on slots and a `CZ(c, Q(h))`: Cliffords only.
//!
//! **Cost per copy** (Toffolis): `1 + B` (fold) `+ 2` (group bits) `+ n' (1 + 2)` (iteration and the
//! two gated lines over `n' = ceil(n_i / 2)` leaves) `+ n' ceil(1.5 w)` (triples) `+ <= w` (cross) `+ 2`
//! (group bits undone) `+ 1` (`d` remade to erase `nu` by measurement with `CZ(d, P_t)` fixups).
//! Qubits: `+ B + 1` against `itemhot`'s read (the folded slots and one more gated line).
use super::itemhot::{ItemData, ItemHot};
use super::onehot::fault_is;
use crate::circuit::{Builder, Qubit};
use crate::walk::common::unary::{erase_and, iterate};

/// The fold's geometry: `e1` slots of the item one-hot, `B = ceil(e1 / 3)` folded slots.
#[derive(Clone, Copy, Debug)]
pub struct Fold {
    pub e1: usize,
    pub b: usize,
}

impl Fold {
    #[must_use]
    pub fn new(e1: usize) -> Self {
        Self {
            e1,
            b: e1.div_ceil(3),
        }
    }

    /// The slots of block `k` (`k B .. min((k + 1) B, e1)`).
    fn block(self, k: usize) -> std::ops::Range<usize> {
        (k * self.b).min(self.e1)..((k + 1) * self.b).min(self.e1)
    }

    /// The old slots that fold into `nu_t`.
    fn members(self, t: usize) -> impl Iterator<Item = usize> {
        let (b, e1) = (self.b, self.e1);
        (0..3).map(move |k| t + k * b).filter(move |&s| s < e1)
    }
}

/// XORs the parity of `set` (qubit indices into `q`) into `q[set[0]]` (in place); call again to
/// undo. Returns the accumulator, `None` for an empty set.
fn accumulate(b: &mut Builder, q: &[Qubit], set: &[usize]) -> Option<Qubit> {
    let (&first, rest) = set.split_first()?;
    for &x in rest {
        b.cx(q[x], q[first]);
    }
    Some(q[first])
}

/// The inner read from the folded one-hot: XORs `data(item, i)` into `out` (`w <= 63` bits) for
/// the lane's item and inner index `i < n_i` (nothing where the one-hot is empty or, with `root`,
/// where `root` is 0), and applies `(-1)^(phase(item, i))` in the same pass (gated the same way).
/// `i_reg[0]` is `i_0`; the iteration runs over `i_reg[1..]`.
///
/// # Panics
/// Unless `hot` is the two-group aligned one-hot (levers `H V`) and `i_reg` has at least two bits.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn read_into_fold(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    out: &[Qubit],
    data: &ItemData<'_>,
    root: Option<Qubit>,
    phase: Option<&dyn Fn(u64, u64) -> bool>,
) {
    assert!(
        hot.aligned && hot.shift == 1,
        "lever - folds the two-group aligned item one-hot (H V)"
    );
    assert!(
        i_reg.len() >= 2,
        "lever -: the inner index needs i_0 and more"
    );
    let w = out.len();
    assert!(w <= 63, "lever -: words up to 63 bits");
    let fd = Fold::new(hot.e1);
    let (e1, bb) = (fd.e1, fd.b);
    let g = hot.g;
    let i0 = i_reg[0];
    let n_half = n_i.div_ceil(2);

    // The fold: d = g AND i_0, nu_t = d AND P_t(h), d erased at once.
    let d = b.alloc();
    b.ccx(g, i0, d);
    let nu = b.alloc_n(bb);
    for (t, &v) in nu.iter().enumerate() {
        let set: Vec<usize> = fd.members(t).collect();
        let acc = accumulate(b, &hot.q, &set).expect("block 0 is never empty");
        if !fault_is(150) {
            b.ccx(d, acc, v);
        }
        accumulate(b, &hot.q, &set);
    }
    erase_and(b, g, i0, d, false);
    // Extended slots: the item one-hot, then nu.
    let ext: Vec<Qubit> = hot.q.iter().chain(&nu).copied().collect();
    let nx = ext.len();

    // c = parity(nu) into nu[0] (in place) for the group bits and the cross fixups.
    let c_on = |b: &mut Builder| {
        for &v in &nu[1..] {
            b.cx(v, nu[0]);
        }
    };
    let blocks: [Vec<usize>; 2] = [fd.block(1).collect(), fd.block(2).collect()];
    let group_bits = |b: &mut Builder, undo: bool| {
        c_on(b);
        let c = nu[0];
        for (k, (&tgt, set)) in [g, i0].iter().zip(&blocks).enumerate() {
            if undo {
                if let Some(acc) = accumulate(b, &hot.q, set) {
                    b.ccx(c, acc, tgt);
                    accumulate(b, &hot.q, set);
                }
                b.cx(c, tgt);
            } else {
                b.cx(c, tgt);
                if let Some(acc) = accumulate(b, &hot.q, set) {
                    if !(fault_is(151) && k == 1) {
                        b.ccx(c, acc, tgt);
                    }
                    accumulate(b, &hot.q, set);
                }
            }
        }
        c_on(b);
    };
    group_bits(b, false);

    // Coefficients. W(grp, s, i): the word of the item at (slot s, group grp), 0 past n_i.
    let wd = |grp: usize, s: usize, i: u64| -> u64 {
        if i >= n_i {
            return 0;
        }
        hot.item(grp, s).map_or(0, |it| data(it, i)[0])
    };
    let ph = |grp: usize, s: usize, i: u64| -> bool {
        i < n_i && phase.is_some_and(|f| hot.item(grp, s).is_some_and(|it| f(it, i)))
    };
    // coef[j][x] for leaf i': bit k of the word, bit 63 the phase.
    let coef = |ip: u64| -> [Vec<u64>; 3] {
        let (lo, hi) = (2 * ip, 2 * ip + 1);
        let pw = |grp: usize, s: usize, i: u64| -> u64 {
            wd(grp, s, i) | u64::from(ph(grp, s, i)) << 63
        };
        let mut c: [Vec<u64>; 3] = [vec![0; nx], vec![0; nx], vec![0; nx]];
        // (Three rows are indexed at once.)
        #[allow(clippy::needless_range_loop)]
        for s in 0..e1 {
            let [c0, c1, c2] = &mut c;
            c0[s] = pw(0, s, lo);
            c1[s] = pw(1, s, lo);
            c2[s] = pw(0, s, hi);
        }
        for t in 0..bb {
            for (k, cv) in c.iter_mut().enumerate() {
                let s = t + k * bb;
                if s < e1 {
                    let folded = pw(1, s, hi);
                    cv[e1 + t] = if fault_is(152) {
                        folded
                    } else {
                        folded ^ cv[s]
                    };
                }
            }
        }
        c
    };

    // Left-overs: lin[k][x] (bit 63 = the phase), cross[k][s].
    let mut lin = vec![0u64; nx];
    let mut cross = vec![0u64; e1];
    let pair_of = |s: usize| e1 + s % bb;
    let mut note = |xs: &[u64], ys: &[u64], mask: u64| {
        for x in 0..nx {
            lin[x] ^= xs[x] & ys[x] & mask;
        }
        for s in 0..e1 {
            let t = pair_of(s);
            cross[s] ^= (xs[s] & ys[t] ^ xs[t] & ys[s]) & mask;
        }
    };
    let par = |b: &mut Builder, v: &[u64], k: usize, line: Qubit| {
        for (x, &q) in ext.iter().enumerate() {
            if v[x] >> k & 1 == 1 {
                b.cx(q, line);
            }
        }
    };
    let bitv = |v: &[u64], k: usize| -> Vec<u64> { v.iter().map(|&x| (x >> k & 1) << k).collect() };
    let nz = |v: &[u64], k: usize| v.iter().any(|&x| x >> k & 1 == 1);

    iterate(
        b,
        root,
        &i_reg[1..],
        n_half,
        &mut |b: &mut Builder, ip: u64, f: Qubit| {
            let [ca, cc, cd] = coef(ip);
            // Lines: u1 = f AND g', u2 = (f AND NOT g') AND i_0', u0 = the rest of f.
            let u1 = b.alloc();
            b.ccx(f, g, u1);
            b.cx(u1, f);
            let u2 = b.alloc();
            b.ccx(f, i0, u2);
            b.cx(u2, f);
            let (a, c, dl) = (f, u1, u2);
            // A paired product of bit k: (a' ^ X)(c' ^ Y) into the targets, recording X Y.
            let prod = |b: &mut Builder,
                        l1: Qubit,
                        xs: &[u64],
                        l2: Qubit,
                        ys: &[u64],
                        k: usize,
                        ts: &[Qubit],
                        cz: bool| {
                par(b, xs, k, l1);
                par(b, ys, k, l2);
                if cz {
                    b.cz(l1, l2);
                } else {
                    match ts {
                        [t] => b.ccx(l1, l2, *t),
                        [t1, t2] => {
                            // Lever `Y`'s sandwich: v into both bits, no scratch.
                            b.cx(*t1, *t2);
                            b.ccx(l1, l2, *t1);
                            b.cx(*t1, *t2);
                        }
                        _ => unreachable!(),
                    }
                }
                par(b, xs, k, l1);
                par(b, ys, k, l2);
                (bitv(xs, k), bitv(ys, k))
            };
            // The single `dl AND D(h)` (no left-over: the first factor has no slot part).
            let single = |b: &mut Builder, k: usize, v: &[u64], t: Option<Qubit>| {
                let set: Vec<usize> = (0..nx).filter(|&x| v[x] >> k & 1 == 1).collect();
                if let Some(acc) = accumulate(b, &ext, &set) {
                    match t {
                        Some(t) => {
                            if !fault_is(153) {
                                b.ccx(dl, acc, t);
                            }
                        }
                        None => b.cz(dl, acc),
                    }
                    accumulate(b, &ext, &set);
                }
            };
            let xorv = |p: &[u64], q: &[u64]| -> Vec<u64> {
                p.iter().zip(q).map(|(x, y)| x ^ y).collect()
            };
            // The phase (bit 63): the pair (a ^ C)(c ^ A) as a CZ, the single d as a CZ.
            if nz(&ca, 63) || nz(&cc, 63) {
                let (x, y) = prod(b, a, &cc, c, &ca, 63, &[], true);
                note(&x, &y, 1 << 63);
            }
            single(b, 63, &cd, None);
            // The word bits: shared triples on the bits that need both the pair and the single.
            let full = |k: usize| (nz(&ca, k) || nz(&cc, k)) && nz(&cd, k);
            let mut ks: Vec<usize> = (0..w).filter(|&k| full(k)).collect();
            if ks.len() % 2 == 1 {
                ks.pop();
            }
            let rest: Vec<usize> = (0..w).filter(|k| !ks.contains(k)).collect();
            for pr in ks.chunks(2) {
                let (k1, k2) = (pr[0], pr[1]);
                // Bit k2's group vectors shifted onto bit k1's position (prod reads bit k1).
                let sh = |v: &[u64]| -> Vec<u64> {
                    v.iter()
                        .map(|&x| (x >> k2 & 1) << k1 | x & !(1 << k1))
                        .collect()
                };
                let (a2, c2, d2) = (sh(&ca), sh(&cc), sh(&cd));
                // v1 = (a ^ d ^ C1)(c ^ A2) into both bits: a: A2, c: C1, d: A2.
                b.cx(dl, a);
                let (x, y) = prod(b, a, &cc, c, &a2, k1, &[out[k1], out[k2]], false);
                b.cx(dl, a);
                note(&x, &y, 1 << k1);
                note(&shift_bit(&x, k1, k2), &shift_bit(&y, k1, k2), 1 << k2);
                // v2 = (a ^ D1 ^ A2)(d ^ A1 ^ A2) into k1: a: A1 ^ A2, d: D1 ^ A2.
                let (x, y) = prod(
                    b,
                    a,
                    &xorv(&cd, &a2),
                    dl,
                    &xorv(&ca, &a2),
                    k1,
                    &[out[k1]],
                    false,
                );
                note(&x, &y, 1 << k1);
                // v3 = (c ^ D2 ^ A2)(d ^ C1 ^ C2) into k2: c: C1 ^ C2, d: D2 ^ A2.
                let (x, y) = prod(
                    b,
                    c,
                    &xorv(&d2, &a2),
                    dl,
                    &xorv(&cc, &c2),
                    k1,
                    &[out[k2]],
                    false,
                );
                note(&shift_bit(&x, k1, k2), &shift_bit(&y, k1, k2), 1 << k2);
            }
            for &k in &rest {
                if nz(&ca, k) || nz(&cc, k) {
                    // (a ^ C)(c ^ A): a: A, c: C.
                    let (x, y) = prod(b, a, &cc, c, &ca, k, &[out[k]], false);
                    note(&x, &y, 1 << k);
                }
                single(b, k, &cd, Some(out[k]));
            }
            b.cx(u2, f);
            erase_and(b, f, i0, u2, false);
            b.cx(u1, f);
            erase_and(b, f, g, u1, false);
        },
    );

    // Linear left-overs: CNOT fan-out (word bits), Z (phase), from every slot.
    for (x, &q) in ext.iter().enumerate() {
        for (k, &o) in out.iter().enumerate() {
            if lin[x] >> k & 1 == 1 && !fault_is(154) {
                b.cx(q, o);
            }
        }
        if lin[x] >> 63 & 1 == 1 {
            b.z(q);
        }
    }
    // Cross left-overs on the folded lanes: (c, Q_k(h)) per bit, CZ(c, Q(h)) for the phase.
    c_on(b);
    for k in (0..w).chain(std::iter::once(63)) {
        let set: Vec<usize> = (0..e1).filter(|&s| cross[s] >> k & 1 == 1).collect();
        if let Some(acc) = accumulate(b, &hot.q, &set) {
            if !fault_is(155) {
                if k == 63 {
                    b.cz(nu[0], acc);
                } else {
                    b.ccx(nu[0], acc, out[k]);
                }
            }
            accumulate(b, &hot.q, &set);
        }
    }
    c_on(b);

    // Undo: the group bits, then nu by measurement (d remade for the CZ(d, P_t) fixups).
    group_bits(b, true);
    let d = b.alloc();
    b.ccx(g, i0, d);
    for (t, &v) in nu.iter().enumerate() {
        let m = b.hmr(v);
        let set: Vec<usize> = fd.members(t).collect();
        let acc = accumulate(b, &hot.q, &set).expect("block 0 is never empty");
        if !fault_is(156) {
            b.cz_if(d, acc, m);
        }
        accumulate(b, &hot.q, &set);
    }
    erase_and(b, g, i0, d, false);
}

/// Moves bit `from` of every entry to bit `to` (the other bits cleared).
fn shift_bit(v: &[u64], from: usize, to: usize) -> Vec<u64> {
    v.iter().map(|&x| (x >> from & 1) << to).collect()
}

#[cfg(test)]
mod tests {
    //! Exhaustive over every item (in and out of range), inner index, root value and three
    //! outcome seeds of small random tables: the folded read gives `root . word`, the lane ends
    //! clean with phase exactly `(-1)^(root . p(item, i))` (the erasure is `itemhot`'s rooted
    //! phase pass on the unfolded one-hot), and every mutant 150-156 is caught.
    use super::*;
    use crate::circuit::Bit;
    use crate::walk::sa_low::itemhot::{erase, phase_rooted, write};
    use crate::walk::sa_low::onehot::FAULT;
    use crate::walk::sa_low::qroam::Split;
    use crate::walk::shared::lookup::{word, Word};
    use crate::walk::shared::testsim::Sim;

    fn table(seed: u64, items: usize, n_i: usize, w: usize) -> Vec<Vec<u64>> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..items)
            .map(|_| {
                (0..n_i)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x % (1 << w)
                    })
                    .collect()
            })
            .collect()
    }

    /// One lane: item `v`, inner index `i`, root; returns (the word read, clean with the
    /// expected phase removed, Toffolis in the read).
    #[allow(clippy::too_many_arguments)]
    fn lane(
        n: usize,
        start: u64,
        n_i: u64,
        w: usize,
        tab: &[Vec<u64>],
        v: u64,
        i: u64,
        root_on: Option<bool>,
        seed: u64,
        fault: u8,
    ) -> (u64, bool, usize) {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let item = b.alloc_n(6);
        let ireg = b.alloc_n(4);
        let root = b.alloc();
        let pq = b.alloc();
        let hot = ItemHot::alloc_aligned(&mut b, start, n, item[0]);
        let in_range = |it: u64| it >= start && it < start + n as u64;
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(w - 1);
            if in_range(it) && ii < n_i {
                wd[0] = tab[(it - start) as usize][ii as usize] & ((1 << (w - 1)) - 1);
            }
            wd
        };
        let ph = |it: u64, ii: u64| {
            in_range(it) && ii < n_i && tab[(it - start) as usize][ii as usize] >> (w - 1) & 1 == 1
        };
        FAULT.with(|c| c.set(fault));
        write(&mut b, &hot, &item);
        let out = b.alloc_n(w - 1);
        let r = root_on.map(|_| root);
        let from = b.ops().len();
        read_into_fold(&mut b, &hot, &ireg, n_i, &out, &data, r, Some(&ph));
        let mid = b.ops().len();
        let tof = b.ops()[from..mid]
            .iter()
            .filter(|o| matches!(o.kind, crate::circuit::OperationType::CCX))
            .count();
        let m: Vec<Bit> = out.iter().map(|&q| b.hmr(q)).collect();
        FAULT.with(|c| c.set(0));
        phase_rooted(&mut b, &hot, &ireg, n_i, &[(&m, &data)], r);
        // The in-pass phase is cancelled by a reference Z on pq.
        erase(&mut b, &hot, &item, 64, Split { h: 2, hot: None });
        b.z(pq);
        let mut sim = Sim::new(&b, seed);
        for (j, &q) in item.iter().enumerate() {
            sim.set(q, v >> j & 1 == 1);
        }
        for (j, &q) in ireg.iter().enumerate() {
            sim.set(q, i >> j & 1 == 1);
        }
        let on = root_on.unwrap_or(true);
        sim.set(root, on);
        sim.set(pq, on && ph(v, i));
        sim.run(&b.ops()[..mid]);
        let got = sim.read(&out);
        sim.run(&b.ops()[mid..]);
        for &q in item.iter().chain(&ireg).chain([&root, &pq]) {
            sim.set(q, false);
        }
        let clean = std::panic::catch_unwind(|| sim.assert_clean()).is_ok();
        (got, clean, tof)
    }

    fn want(n: usize, start: u64, n_i: u64, w: usize, tab: &[Vec<u64>], v: u64, i: u64) -> u64 {
        if v >= start && v < start + n as u64 && i < n_i {
            tab[(v - start) as usize][i as usize] & ((1 << (w - 1)) - 1)
        } else {
            0
        }
    }

    #[test]
    fn folded_read_is_exact_for_every_lane() {
        // (n items from start): e1 from 2 to 12 slots, odd and even boundaries; blocks of 1-4.
        let cases = [
            (3usize, 1u64),
            (4, 0),
            (5, 3),
            (6, 0),
            (9, 7),
            (13, 2),
            (16, 5),
            (21, 8),
            (24, 0),
        ];
        for (n, start) in cases {
            for n_i in [3u64, 8, 13, 16] {
                let w = 7;
                let tab = table(n as u64 * 17 + n_i, n, 16, w);
                for v in 0..40u64 {
                    for i in 0..16u64 {
                        for root_on in [None, Some(false), Some(true)] {
                            for seed in 1..3 {
                                let (got, clean, _) =
                                    lane(n, start, n_i, w, &tab, v, i, root_on, seed, 0);
                                let exp = if root_on == Some(false) {
                                    0
                                } else {
                                    want(n, start, n_i, w, &tab, v, i)
                                };
                                assert_eq!(
                                    got, exp,
                                    "n {n} start {start} n_i {n_i} item {v} i {i} root {root_on:?}"
                                );
                                assert!(
                                    clean,
                                    "n {n} start {start} n_i {n_i} item {v} i {i} root {root_on:?} seed {seed}: dirty"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn folded_read_mutants_are_caught() {
        let (n, start, n_i) = (21usize, 8u64, 16u64);
        for fault in 150u8..=156 {
            // w 8 leaves one word bit outside the shared triples (the single of fault 153).
            let caught = [7usize, 8].iter().any(|&w| {
                let tab = table(11 + w as u64, n, 16, w);
                (0..40u64).any(|v| {
                    (0..16u64).any(|i| {
                        [None, Some(true)].iter().any(|&root_on| {
                            (1..3).any(|seed| {
                                let (got, clean, _) =
                                    lane(n, start, n_i, w, &tab, v, i, root_on, seed, fault);
                                got != want(n, start, n_i, w, &tab, v, i) || !clean
                            })
                        })
                    })
                })
            });
            assert!(caught, "fold fault {fault} not caught");
        }
    }

    /// The fold pays: on a 64-index table over 285 items (Li's shape) the folded read costs about
    /// `1.5 w` per bit per leaf over 32 leaves, plus `B + O(w)`, against `itemhot`'s `w + 2` over 64.
    #[test]
    fn folded_read_costs() {
        let count = |fold: bool, w: usize| {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let item = b.alloc_n(9);
            let ireg = b.alloc_n(6);
            let hot = ItemHot::alloc_aligned(&mut b, 76, 285, item[0]);
            let tab = table(3, 285, 64, w);
            let data = |it: u64, ii: u64| -> Word {
                let mut wd = word(w);
                if (76..361).contains(&it) {
                    wd[0] = tab[(it - 76) as usize][ii as usize];
                }
                wd
            };
            let out = b.alloc_n(w);
            let from = b.ops().len();
            if fold {
                read_into_fold(&mut b, &hot, &ireg, 64, &out, &data, None, None);
            } else {
                super::super::itemhot::read_into(&mut b, &hot, &ireg, 64, &out, &data);
            }
            b.ops()[from..]
                .iter()
                .filter(|o| matches!(o.kind, crate::circuit::OperationType::CCX))
                .count()
        };
        for w in [8usize, 15, 16] {
            let (old, new) = (count(false, w), count(true, w));
            println!("w {w}: item one-hot read {old}, folded {new} Toffolis");
            assert!(new < old, "the fold must pay at w {w}");
        }
    }
}
