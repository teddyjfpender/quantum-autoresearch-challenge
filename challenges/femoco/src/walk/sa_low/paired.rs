//! The paired unary lookup (`sa-toff` lever `G`): a table read whose
//! cost per output bit is half a Toffoli per *group* of `2^s` entries, against one Toffoli per
//! block of `lambda` entries per *word* for clean QROAM.
//!
//! **Layout.** The index `x` (little-endian) is split into its low `s` bits, the *slot* `v`, and
//! the rest, the *group* `g`: `x = g 2^s + v`. The slot is written one-hot into `2^s` fresh
//! qubits `h` (unary iteration, `2^s - 2` Toffolis). The table is then a matrix
//! `W(g, v) = data(g 2^s + v)` and output bit `k` is `sum_g [x_hi = g] W_k(g, h)`, with
//! `W_k(g, h)` the parity of the slots `v` whose word has bit `k` set: a CNOT fan-out of `h`.
//!
//! **Pairing.** The group indicators come from a unary iteration over the high bits. At its last
//! level the two siblings `l = [x_hi = 2m]` and `r = [x_hi = 2m + 1]` are live at once (the
//! parent flag `c` is turned into `l` in place by `c ^= r`), and they are never both 1. So
//! `(l ^ X(h)) (r ^ Y(h)) = l Y(h) ^ r X(h) ^ X(h) Y(h)` with `Y = W_k(2m, .)`,
//! `X = W_k(2m + 1, .)`: one Toffoli gives both groups' contribution to bit `k`, and the
//! leftover `X Y` is a function of the one-hot alone. It is added on every lane, whatever the
//! group, so one CNOT fan-out of `sum_m X_m Y_m` (per bit) after the iteration cancels all of
//! them. (The parities are XORed into `l` and `r` in place and undone; a parity over more than
//! half the slots is taken as the complement plus an `X`, since exactly one slot is set.)
//! This is the paired group correction (lever `z`, two one-hot group bits never
//! both 1) applied to the sibling flags of a unary iteration, so the group register is never
//! stored.
//!
//! **Erasure.** The slot one-hot is X-measured; outcomes `m` leave `(-1)^(m[v])` on every lane
//! (whether its index is in the table's range or not), cancelled by a phase fixup over the low
//! `s` bits alone (`qroam::fixup`). The output holds `data(x)` on lanes with `start <= x <
//! limit` and 0 elsewhere, exactly what `qroam::load_range` leaves, with no junk: the returned
//! [`Read`] has one block, so the walk's erasures (`qroam::erase_parts`, `erase_split_by`) apply
//! unchanged.
//!
//! **Cost.** `2^s - 2` (slot one-hot) + about `G` (the iteration over `G = ceil(L / 2^s)`
//! groups) + at most `w ceil(G / 2)` (the pairs; a pair whose two parities are both empty costs
//! nothing, and an unpaired group costs one Toffoli per bit with a nonempty parity) + the low-bit
//! fixup. Qubits: `2^s + w` plus the iteration's AND ladder and one scratch. Against clean QROAM
//! with `lambda` blocks (`L / lambda + (lambda - 1) w` Toffolis, `lambda w` qubits), at equal
//! qubits (`2^s ~ lambda w`) the leading term `w L / 2^s` is halved; the price is the slot
//! one-hot's write, which grows with `2^s`.
use super::qroam::{self, best_split, Read};
use crate::circuit::{Bit, Builder, Qubit};
use crate::walk::shared::lookup::{measure, word, Data, Word};
use crate::walk::shared::unary::{erase_and, iterate};

// Deliberate faults for the mutant tests: 0 none, 40 the leftover `X Y` fan-out skipped, 41 the
// two parities swapped between the sibling flags, 42 the slot one-hot's phase fixup skipped,
// 43 (hosting) an index bit not rebuilt from the one-hot, 44 (hosting) a hosted slot
// not moved back before the one-hot is measured, 45 (collapse) a collapse `CZ` skipped, 46
// (`phase_paired`) the last outcome bit's phase skipped.
#[cfg(test)]
thread_local! {
    pub(super) static FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

fn fault_is(k: u8) -> bool {
    #[cfg(test)]
    {
        FAULT.with(std::cell::Cell::get) == k
    }
    #[cfg(not(test))]
    {
        let _ = k;
        false
    }
}

/// Called at each sibling pair `(2m, 2m + 1)` of the group iteration with `2m` and the two
/// indicators (`None` for a group outside the range).
pub(super) type Pair<'a> = dyn FnMut(&mut Builder, u64, Option<Qubit>, Option<Qubit>) + 'a;

/// Unary iteration over `index` for the values `lo..hi` that hands out sibling leaves in pairs.
pub(super) fn iterate_pairs(
    b: &mut Builder,
    root: Option<Qubit>,
    index: &[Qubit],
    lo: u64,
    hi: u64,
    pair: &mut Pair<'_>,
) {
    assert!(!index.is_empty(), "nothing to iterate on");
    if lo < hi {
        node(b, root, index, 0, lo, hi, pair);
    }
}

fn node(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    base: u64,
    lo: u64,
    hi: u64,
    pair: &mut Pair<'_>,
) {
    let (&bit, low) = index.split_last().expect("pairs need a bit");
    let half = 1u64 << low.len();
    let left = base + half > lo && base < hi;
    let right = base + half < hi && base + 2 * half > lo;
    if low.is_empty() {
        // The pair level: siblings `base` (bit 0) and `base + 1` (bit 1).
        match ctl {
            None => {
                // Only one index bit in all: `r` is the bit itself, `l` its negation.
                let l = b.alloc();
                b.cx(bit, l);
                b.x(l);
                pair(b, base, left.then_some(l), right.then_some(bit));
                b.x(l);
                b.cx(bit, l);
                b.free(l);
            }
            Some(c) => match (left, right) {
                (true, true) => {
                    let r = b.alloc();
                    b.ccx(c, bit, r);
                    b.cx(r, c);
                    pair(b, base, Some(c), Some(r));
                    b.cx(r, c);
                    erase_and(b, c, bit, r, false);
                }
                (true, false) => {
                    let l = b.alloc();
                    b.x(bit);
                    b.ccx(c, bit, l);
                    b.x(bit);
                    pair(b, base, Some(l), None);
                    erase_and(b, c, bit, l, true);
                }
                (false, true) => {
                    let r = b.alloc();
                    b.ccx(c, bit, r);
                    pair(b, base, None, Some(r));
                    erase_and(b, c, bit, r, false);
                }
                (false, false) => {}
            },
        }
        return;
    }
    let Some(c) = ctl else {
        if left {
            b.x(bit);
            node(b, Some(bit), low, base, lo, hi, pair);
            b.x(bit);
        }
        if right {
            node(b, Some(bit), low, base + half, lo, hi, pair);
        }
        return;
    };
    let child = b.alloc();
    if left {
        b.x(bit);
        b.ccx(c, bit, child);
        b.x(bit);
        node(b, Some(child), low, base, lo, hi, pair);
        if right {
            b.cx(c, child);
            node(b, Some(child), low, base + half, lo, hi, pair);
            erase_and(b, c, bit, child, false);
        } else {
            erase_and(b, c, bit, child, true);
        }
    } else if right {
        b.ccx(c, bit, child);
        node(b, Some(child), low, base + half, lo, hi, pair);
        erase_and(b, c, bit, child, false);
    } else {
        b.free(child);
    }
}

/// The slots of `set` as qubits of `hot`, or (when that is shorter) the other slots plus a flag
/// that the parity is complemented: exactly one slot is set on every lane, so the parity of a
/// set is 1 minus the parity of its complement.
fn parity_support(hot: &[Qubit], set: &[bool]) -> (Vec<Qubit>, bool) {
    let n = set.iter().filter(|&&x| x).count();
    let flip = 2 * n > hot.len();
    let qs = hot
        .iter()
        .zip(set)
        .filter(|(_, &x)| x != flip)
        .map(|(&h, _)| h)
        .collect();
    (qs, flip)
}

/// XORs the parity of `set` (over the one-hot `hot`) into `q`.
fn xor_parity(b: &mut Builder, hot: &[Qubit], set: &[bool], q: Qubit) {
    let (qs, flip) = parity_support(hot, set);
    qs.iter().for_each(|&h| b.cx(h, q));
    if flip {
        b.x(q);
    }
}

/// `out ^= f AND parity(set)(hot)`: one Toffoli, through a scratch qubit unless the set is a
/// single slot.
fn single(b: &mut Builder, hot: &[Qubit], f: Qubit, set: &[bool], out: Qubit) {
    let n = set.iter().filter(|&&x| x).count();
    if n == 0 {
        return;
    }
    if n == 1 {
        let v = set.iter().position(|&x| x).expect("one slot");
        b.ccx(f, hot[v], out);
        return;
    }
    let s = b.alloc();
    xor_parity(b, hot, set, s);
    b.ccx(f, s, out);
    xor_parity(b, hot, set, s);
    b.free(s);
}

/// Options of [`load_paired`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    /// Lever `U`: the low index qubits host `s` slots while the pairs run.
    pub host: bool,
    /// Lever `n`: the slot one-hot is written by an in-place split expansion (`2^s - 1` ANDs) and
    /// erased by the measured collapse ([`collapse`]): no phase fixup over the low bits.
    pub collapse: bool,
    /// Lever `.p` (with `collapse`): the slot one-hot is a pruned expansion
    /// ([`load_paired_pruned`]): a subcube of slot values on which every group's entry (word and
    /// in-pass phase) is the same is one slot, not split further.
    pub prune: bool,
}

/// Lever `n`: writes the one-hot of `low` into `hot` (all `|0>`) by splits: the root (slot 0) is
/// set to 1, then for each index bit from the top, every node `v` splits into `v` and
/// `v + 2^k` by `child = node AND x_k`, `node ^= child`. `2^s - 1` Toffolis; `low` is read only.
fn expand(b: &mut Builder, hot: &[Qubit], low: &[Qubit]) {
    b.x(hot[0]);
    for k in (0..low.len()).rev() {
        let step = 1usize << (k + 1);
        for v in (0..hot.len()).step_by(step) {
            let c = v + (1 << k);
            b.ccx(hot[v], low[k], hot[c]);
            b.cx(hot[c], hot[v]);
        }
    }
}

/// Lever `n`: the measured collapse of [`expand`] (as lever `I`'s):
/// splits are undone from index bit 0 up; each child `c = node AND x_k` is merged back
/// (`node ^= c`), X-measured, and an outcome of 1 is cancelled by `CZ(node, x_k)`, both still
/// present. Cliffords only; the root is reset by `X` at the end. `low` must hold the index.
fn collapse(b: &mut Builder, hot: &[Qubit], low: &[Qubit]) {
    for (k, &x) in low.iter().enumerate() {
        let step = 1usize << (k + 1);
        for v in (0..hot.len()).step_by(step) {
            let c = v + (1 << k);
            b.cx(hot[c], hot[v]);
            let m = b.hmr(hot[c]);
            if !fault_is(45) {
                b.cz_if(hot[v], x, m);
            }
        }
    }
    b.x(hot[0]);
    b.free(hot[0]);
}

/// XORs bit `k` of the slot value (the parity of the slots whose value has bit `k` set) into `q`.
fn xor_bit(b: &mut Builder, hot: &[Qubit], k: usize, q: Qubit) {
    for (v, &h) in hot.iter().enumerate() {
        if v >> k & 1 == 1 {
            b.cx(h, q);
        }
    }
}

/// Loads `data(x)` for the lane's `index` value `x` in `start..limit` (0 elsewhere) into a fresh
/// `w`-qubit register with the paired unary lookup over a one-hot of the low `s` index bits.
///
/// **Hosting** (`host`, lever `M`): once the slot one-hot is written, each low index
/// bit `x_k` is a parity of it (`x_k = XOR of the slots whose value has bit k set`, exactly one
/// slot being 1), so it is XORed to zero by CNOTs and its qubit takes one slot's content by a
/// `Swap`; the emptied fresh slot qubit is freed. The low index qubits (uniform or ancilla) thus
/// carry `s` of the `2^s` slots while the pairs run: `s` fewer live qubits, 0 Toffolis. Before the
/// one-hot is measured each hosted slot is swapped into a fresh qubit and the index bit is
/// rebuilt by the same parity, so the measurement and its fixup touch only ancillas and the
/// index ends as it began.
///
/// # Panics
/// If `s` leaves no high index bits.
#[allow(clippy::too_many_arguments)]
pub fn load_paired(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &Data<'_>,
    s: usize,
    opt: Opts,
) -> Read {
    load_paired_ext(b, index, start, limit, w, data, s, opt, None, None)
}

/// [`load_paired`] with two options (lever `t` on the
/// paired lookup; both `None` emits exactly [`load_paired`]'s ops):
///
/// - `root`: the group iteration is rooted at this qubit (the walk control): where it is 0 every
///   group flag is 0, so the output stays 0 and no phase is applied;
/// - `phase`: the lane phase `(-1)^(phase(x))` applied in the same pass with Cliffords only: at
///   each group's flag `f`, `CZ(f, h_v)` for every slot `v` of that group whose entry has it set
///   (`f . parity(h)`, `h` exactly one-hot), so nothing is left over.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn load_paired_ext(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &Data<'_>,
    s: usize,
    opt: Opts,
    root: Option<Qubit>,
    phase: Option<&dyn Fn(u64) -> bool>,
) -> Read {
    let host = opt.host;
    assert!(
        s >= 1 && s < index.len(),
        "the paired lookup needs slot and group bits"
    );
    let (low, high) = index.split_at(s);
    let lam = 1u64 << s;
    let hot = b.alloc_n(lam as usize);
    if opt.collapse {
        expand(b, &hot, low);
    } else {
        let mut leaf = |b: &mut Builder, v: u64, flag: Qubit| b.cx(flag, hot[v as usize]);
        iterate(b, None, low, lam, &mut leaf);
    }
    // Hosting: index qubit `k` carries the slot `2^k` (whose value has bit `k` alone; slot `2^j`,
    // `j != k`, is never in bit `k`'s parity set, so the order does not matter).
    let mut hot = hot;
    let hosted: Vec<(usize, usize)> = if host {
        (0..s).map(|k| (k, 1usize << k)).collect()
    } else {
        Vec::new()
    };
    for &(k, v) in &hosted {
        xor_bit(b, &hot, k, low[k]);
        b.swap(low[k], hot[v]);
        b.free(hot[v]);
        hot[v] = low[k];
    }
    let out = b.alloc_n(w);
    let at = |g: u64, v: u64| -> Word {
        let x = g * lam + v;
        if (start..limit).contains(&x) {
            data(x)
        } else {
            word(w)
        }
    };
    // `bits[g][k][v]`: bit `k` of the word at group `g`, slot `v`.
    let column = |g: u64| -> Vec<Vec<bool>> {
        let words: Vec<Word> = (0..lam).map(|v| at(g, v)).collect();
        (0..w)
            .map(|k| {
                words
                    .iter()
                    .map(|d| d[k / 64] >> (k % 64) & 1 == 1)
                    .collect()
            })
            .collect()
    };
    let mut garbage = vec![vec![false; lam as usize]; w];
    {
        let mut pair = |b: &mut Builder, g: u64, l: Option<Qubit>, r: Option<Qubit>| {
            if let Some(ph) = phase {
                for (gg, f) in [(g, l), (g + 1, r)] {
                    let Some(f) = f else { continue };
                    for v in 0..lam {
                        let x = gg * lam + v;
                        if (start..limit).contains(&x) && ph(x) && !fault_is(110) {
                            b.cz(f, hot[v as usize]);
                        }
                    }
                }
            }
            let yl = l.map(|_| column(g));
            let xr = r.map(|_| column(g + 1));
            for (k, &q) in out.iter().enumerate() {
                let y = yl.as_ref().map(|c| &c[k][..]);
                let x = xr.as_ref().map(|c| &c[k][..]);
                let ny = y.is_some_and(|c| c.iter().any(|&t| t));
                let nx = x.is_some_and(|c| c.iter().any(|&t| t));
                match (ny, nx) {
                    (false, false) => {}
                    (true, false) => single(b, &hot, l.expect("left"), y.expect("left"), q),
                    (false, true) => single(b, &hot, r.expect("right"), x.expect("right"), q),
                    (true, true) => {
                        let (lq, rq) = (l.expect("left"), r.expect("right"));
                        let (y, x) = (y.expect("left"), x.expect("right"));
                        let (xa, ya) = if fault_is(41) { (y, x) } else { (x, y) };
                        xor_parity(b, &hot, xa, lq);
                        xor_parity(b, &hot, ya, rq);
                        b.ccx(lq, rq, q);
                        xor_parity(b, &hot, xa, lq);
                        xor_parity(b, &hot, ya, rq);
                        for (v, gv) in garbage[k].iter_mut().enumerate() {
                            *gv ^= x[v] && y[v];
                        }
                    }
                }
            }
        };
        let root = if fault_is(111) { None } else { root };
        iterate_pairs(b, root, high, start / lam, limit.div_ceil(lam), &mut pair);
    }
    if !fault_is(40) {
        for (k, &q) in out.iter().enumerate() {
            for (v, &gv) in garbage[k].iter().enumerate() {
                if gv {
                    b.cx(hot[v], q);
                }
            }
        }
    }
    for &(k, v) in hosted.iter().rev() {
        if fault_is(44) && k == 0 {
            continue;
        }
        let q = b.alloc();
        b.swap(low[k], q);
        hot[v] = q;
        if !(fault_is(43) && k == 0) {
            xor_bit(b, &hot, k, low[k]);
        }
    }
    if opt.collapse {
        collapse(b, &hot, low);
        return qroam::read_from_parts(out, w, limit, start);
    }
    let mh = measure(b, hot);
    if !fault_is(42) {
        let unit = |v: u64| -> Word {
            let mut u = word(mh.len());
            u[(v / 64) as usize] |= 1 << (v % 64);
            u
        };
        qroam::fixup(b, low, lam, &mh, &unit, best_split(s, lam, true));
    }
    qroam::read_from_parts(out, w, limit, start)
}

/// Lever `.p`: the leaves `(base, size)` of the pruned slot tree over `s` bits, in
/// increasing `base`: a node (an aligned subcube of slot values) is a leaf when it is a single
/// value or `merge(base, size)` says every value in it may share one slot.
#[must_use]
pub fn prune_leaves(s: usize, merge: &dyn Fn(u64, u64) -> bool) -> Vec<(u64, u64)> {
    fn rec(out: &mut Vec<(u64, u64)>, base: u64, size: u64, merge: &dyn Fn(u64, u64) -> bool) {
        if size == 1 || merge(base, size) {
            out.push((base, size));
        } else {
            rec(out, base, size / 2, merge);
            rec(out, base + size / 2, size / 2, merge);
        }
    }
    let mut out = Vec::new();
    rec(&mut out, 0, 1 << s, merge);
    out
}

/// Lever `.p`: writes the pruned slot one-hot of `low` (leaves of [`prune_leaves`]) by splits
/// from the top bit, as [`expand`] but leaving every leaf whole: `leaves - 1` Toffolis. Returns
/// the leaf qubits in leaf order.
fn expand_pruned(b: &mut Builder, low: &[Qubit], leaves: &[(u64, u64)]) -> Vec<Qubit> {
    let s = low.len();
    let root = b.alloc();
    b.x(root);
    let mut front: Vec<(u64, u64, Qubit)> = vec![(0, 1 << s, root)];
    for k in (0..s).rev() {
        let mut next = Vec::with_capacity(front.len() * 2);
        for &(base, size, q) in &front {
            if size == 2 << k && !leaves.contains(&(base, size)) {
                let c = b.alloc();
                b.ccx(q, low[k], c);
                b.cx(c, q);
                next.push((base, size / 2, q));
                next.push((base + size / 2, size / 2, c));
            } else {
                next.push((base, size, q));
            }
        }
        front = next;
    }
    assert_eq!(front.len(), leaves.len(), "the pruned tree has its leaves");
    front
        .iter()
        .zip(leaves)
        .map(|(&(base, size, q), &(lb, ls))| {
            assert_eq!((base, size), (lb, ls), "leaf order");
            q
        })
        .collect()
}

/// Lever `.p`: the measured collapse of [`expand_pruned`] (every split undone from bit 0 up, each
/// child merged back, X-measured and its outcome cancelled by `CZ(node, x_k)`). Frees the root.
fn collapse_pruned(b: &mut Builder, hot: &[Qubit], low: &[Qubit], leaves: &[(u64, u64)]) {
    let s = low.len();
    let mut nodes: Vec<(u64, u64, Qubit)> = leaves
        .iter()
        .zip(hot)
        .map(|(&(base, size), &q)| (base, size, q))
        .collect();
    for (k, &x) in low.iter().enumerate().take(s) {
        let mut next = Vec::with_capacity(nodes.len());
        let mut i = 0;
        while i < nodes.len() {
            let (base, size, q) = nodes[i];
            if size == 1 << k && base % (2 << k) == 0 && i + 1 < nodes.len() {
                let (b2, s2, c) = nodes[i + 1];
                if s2 == size && b2 == base + size {
                    b.cx(c, q);
                    let m = b.hmr(c);
                    if !fault_is(45) {
                        b.cz_if(q, x, m);
                    }
                    next.push((base, 2 * size, q));
                    i += 2;
                    continue;
                }
            }
            next.push((base, size, q));
            i += 1;
        }
        nodes = next;
    }
    assert_eq!(nodes.len(), 1, "the collapse reaches the root");
    b.x(nodes[0].2);
    b.free(nodes[0].2);
}

/// Lever `.p`: [`load_paired_ext`] (with `collapse`) over a pruned slot one-hot. A
/// subcube of slot values whose entries (the word and the in-pass phase, 0 outside
/// `start..limit`) are equal in every group of the iteration is one slot: the read is the same
/// on every lane, since each group's contribution is a parity of its slots' words. Hosting
/// (`host`) keeps only the index bits that are constant on every leaf (bit `k` is a parity of
/// the leaves whose values have it set), each in the leaf holding the value `2^k`.
///
/// Cost: `leaves - 1` (the pruned one-hot) + the group iteration + at most `w` per pair, as
/// [`load_paired`]; qubits `leaves - hosted + w` plus the iteration's ladder and one scratch.
///
/// # Panics
/// If `s` leaves no high index bits, or without `collapse`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn load_paired_pruned(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &Data<'_>,
    s: usize,
    opt: Opts,
    root: Option<Qubit>,
    phase: Option<&dyn Fn(u64) -> bool>,
) -> Read {
    assert!(
        s >= 1 && s < index.len() && opt.collapse,
        "the pruned paired lookup needs slot and group bits and the collapse"
    );
    let (low, high) = index.split_at(s);
    let lam = 1u64 << s;
    let at = |g: u64, v: u64| -> (Word, bool) {
        let x = g * lam + v;
        if (start..limit).contains(&x) {
            (data(x), phase.is_some_and(|ph| ph(x)))
        } else {
            (word(w), false)
        }
    };
    let (g0, g1) = (start / lam, limit.div_ceil(lam));
    let merge = |base: u64, size: u64| -> bool {
        if fault_is(113) && base + size == lam && size > 1 {
            return true;
        }
        (g0..g1).all(|g| {
            let e = at(g, base);
            (base + 1..base + size).all(|v| at(g, v) == e)
        })
    };
    let leaves = prune_leaves(s, &merge);
    let mut hot = expand_pruned(b, low, &leaves);
    let max_leaf = leaves.iter().map(|&(_, z)| z).max().unwrap_or(1);
    let leaf_of = |v: u64| leaves.partition_point(|&(lb, _)| lb <= v) - 1;
    let xor_leaf_bit = |b: &mut Builder, hot: &[Qubit], k: usize, q: Qubit| {
        for (i, &(lb, _)) in leaves.iter().enumerate() {
            if lb >> k & 1 == 1 {
                b.cx(hot[i], q);
            }
        }
    };
    let hosted: Vec<(usize, usize)> = if opt.host {
        (0..s)
            .filter(|&k| max_leaf <= 1 << k)
            .map(|k| (k, leaf_of(1 << k)))
            .collect()
    } else {
        Vec::new()
    };
    for &(k, li) in &hosted {
        xor_leaf_bit(b, &hot, k, low[k]);
        b.swap(low[k], hot[li]);
        b.free(hot[li]);
        hot[li] = low[k];
    }
    let out = b.alloc_n(w);
    let column = |g: u64| -> (Vec<Vec<bool>>, Vec<bool>) {
        let words: Vec<(Word, bool)> = leaves.iter().map(|&(lb, _)| at(g, lb)).collect();
        let bits = (0..w)
            .map(|k| {
                words
                    .iter()
                    .map(|(d, _)| d[k / 64] >> (k % 64) & 1 == 1)
                    .collect()
            })
            .collect();
        (bits, words.iter().map(|(_, p)| *p).collect())
    };
    let nl = leaves.len();
    let mut garbage = vec![vec![false; nl]; w];
    {
        let mut pair = |b: &mut Builder, g: u64, l: Option<Qubit>, r: Option<Qubit>| {
            let yl = l.map(|_| column(g));
            let xr = r.map(|_| column(g + 1));
            if phase.is_some() {
                for (c, f) in [(&yl, l), (&xr, r)] {
                    let (Some((_, ph)), Some(f)) = (c, f) else {
                        continue;
                    };
                    for (i, &p) in ph.iter().enumerate() {
                        if p && !fault_is(110) {
                            b.cz(f, hot[i]);
                        }
                    }
                }
            }
            for (k, &q) in out.iter().enumerate() {
                let y = yl.as_ref().map(|c| &c.0[k][..]);
                let x = xr.as_ref().map(|c| &c.0[k][..]);
                let ny = y.is_some_and(|c| c.iter().any(|&t| t));
                let nx = x.is_some_and(|c| c.iter().any(|&t| t));
                match (ny, nx) {
                    (false, false) => {}
                    (true, false) => single(b, &hot, l.expect("left"), y.expect("left"), q),
                    (false, true) => single(b, &hot, r.expect("right"), x.expect("right"), q),
                    (true, true) => {
                        let (lq, rq) = (l.expect("left"), r.expect("right"));
                        let (y, x) = (y.expect("left"), x.expect("right"));
                        let (xa, ya) = if fault_is(41) { (y, x) } else { (x, y) };
                        xor_parity(b, &hot, xa, lq);
                        xor_parity(b, &hot, ya, rq);
                        b.ccx(lq, rq, q);
                        xor_parity(b, &hot, xa, lq);
                        xor_parity(b, &hot, ya, rq);
                        for (v, gv) in garbage[k].iter_mut().enumerate() {
                            *gv ^= x[v] && y[v];
                        }
                    }
                }
            }
        };
        let root = if fault_is(111) { None } else { root };
        iterate_pairs(b, root, high, g0, g1, &mut pair);
    }
    if !fault_is(40) {
        for (k, &q) in out.iter().enumerate() {
            for (v, &gv) in garbage[k].iter().enumerate() {
                if gv {
                    b.cx(hot[v], q);
                }
            }
        }
    }
    for &(k, li) in hosted.iter().rev() {
        if fault_is(44) && k == hosted[0].0 {
            continue;
        }
        let q = b.alloc();
        b.swap(low[k], q);
        hot[li] = q;
        if !(fault_is(43) && k == hosted[0].0) {
            xor_leaf_bit(b, &hot, k, low[k]);
        }
    }
    collapse_pruned(b, &hot, low, &leaves);
    qroam::read_from_parts(out, w, limit, start)
}

/// Lever `F`: the phase `(-1)^(bits . data(x))` for the lane's `index` value `x` in
/// `start..limit` (no phase elsewhere), the measured erasure of a lookup's output. The low `s`
/// index bits are written one-hot by the split expansion, a range iteration runs over the high
/// bits, and at group `g`'s leaf flag `f` every outcome bit `k` applies
/// `(-1)^(f . parity_k(h))` as conditioned `CZ`s on the slots whose word has bit `k` set (the
/// complement and a `Z` on `f` when that is shorter): Cliffords only. The one-hot leaves by the
/// measured collapse. Toffolis: `2^s - 1` plus the iteration, with `s` chosen for the fewest.
pub fn phase_paired(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    bits: &[Bit],
    data: &Data<'_>,
) {
    phase_paired_rooted(b, index, start, limit, bits, data, None);
}

/// [`phase_paired`] with the range iteration rooted at `root` (lever `^`): the phases are
/// applied only where `root` is 1. `None` emits exactly [`phase_paired`]'s ops.
pub fn phase_paired_rooted(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    bits: &[Bit],
    data: &Data<'_>,
    root: Option<Qubit>,
) {
    let s = (1..index.len())
        .min_by_key(|&s| {
            let lam = 1u64 << s;
            (lam - 1) + super::range::range_cost(index.len() - s, start / lam, limit.div_ceil(lam))
        })
        .expect("the phase needs slot and group bits");
    let (low, high) = index.split_at(s);
    let lam = 1u64 << s;
    let hot = b.alloc_n(lam as usize);
    expand(b, &hot, low);
    let nb = bits.len();
    let mut leaf = |b: &mut Builder, g: u64, f: Qubit| {
        let words: Vec<Option<Word>> = (0..lam)
            .map(|v| {
                let x = g * lam + v;
                (start..limit).contains(&x).then(|| data(x))
            })
            .collect();
        for (k, &m) in bits.iter().enumerate() {
            let set: Vec<bool> = words
                .iter()
                .map(|w| w.as_ref().is_some_and(|w| w[k / 64] >> (k % 64) & 1 == 1))
                .collect();
            if fault_is(46) && k == nb - 1 {
                continue;
            }
            let (qs, flip) = parity_support(&hot, &set);
            if flip {
                b.z_if(f, m);
            }
            for q in qs {
                b.cz_if(f, q, m);
            }
        }
    };
    let root = if fault_is(112) { None } else { root };
    super::range::iterate_range(b, root, high, start / lam, limit.div_ceil(lam), &mut leaf);
    collapse(b, &hot, low);
}

/// Toffolis of [`load_paired`] for a table whose every pair has nonempty parities (an upper
/// bound): the slot one-hot, the group iteration, `w` per pair (or per unpaired group) and the
/// low-bit fixup.
#[must_use]
pub fn paired_cost(bits: usize, start: u64, limit: u64, w: usize, s: usize) -> u64 {
    let lam = 1u64 << s;
    let (lo, hi) = (start / lam, limit.div_ceil(lam));
    let one_hot = qroam::iterate_cost(s, lam);
    let iter = super::range::range_cost(bits - s, lo, hi);
    let pairs = (lo..hi)
        .map(|g| g / 2)
        .collect::<std::collections::BTreeSet<_>>();
    let fix = qroam::fixup_cost(s, lam, best_split(s, lam, true));
    one_hot + iter + pairs.len() as u64 * w as u64 + fix
}

#[cfg(test)]
mod tests {
    //! Exhaustive gadget tests: for every index value of small random tables (ranges with odd
    //! and even ends, every slot width), the read leaves exactly `data(x)` (0 outside the range)
    //! and nothing else, the walk's erasure (`qroam::erase_split_by`, `erase_parts`) returns
    //! every ancilla to `|0>` with phase `+1` over several measurement seeds, the Toffoli count
    //! does not depend on the lane and stays within [`paired_cost`], and each mutant (faults 40,
    //! 41, 42) breaks a lane.
    use super::*;
    use crate::walk::shared::testsim::Sim;

    fn table(seed: u64, n: u64, w: usize) -> Vec<Word> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                vec![x % (1 << w)]
            })
            .collect()
    }

    /// One lane at index `x`: returns the output and the Toffolis, or panics if the erasure
    /// leaves anything behind. `parts` uses `erase_parts` (the lever-`m` path) instead of
    /// `erase_split_by`.
    #[allow(clippy::too_many_arguments)]
    fn lane(
        bits: usize,
        start: u64,
        limit: u64,
        w: usize,
        s: usize,
        tab: &[Word],
        x: u64,
        seed: u64,
        parts: bool,
        opt: Opts,
    ) -> (u64, u64) {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let index = b.alloc_n(bits);
        let data = |y: u64| -> Word {
            if (start..limit).contains(&y) {
                tab[y as usize].clone()
            } else {
                word(w)
            }
        };
        let mut sim = Sim::new(&b, seed);
        for (i, &q) in index.iter().enumerate() {
            sim.set(q, x >> i & 1 == 1);
        }
        let r = load_paired(&mut b, &index, start, limit, w, &data, s, opt);
        sim.run(b.ops());
        let got = sim.read(&r.out);
        let t = sim.toffolis;
        let n0 = b.ops().len();
        if parts {
            let outb: Vec<_> = r.out.iter().map(|&q| b.hmr(q)).collect();
            let none = |_: u64| word(0);
            let split = best_split(bits, limit, true);
            qroam::erase_parts(
                &mut b,
                &index,
                r,
                &data,
                outb,
                &data,
                Vec::new(),
                &none,
                split,
            );
        } else {
            assert_eq!(start, 0);
            qroam::erase_split_by(&mut b, &index, r, &data, best_split(bits, limit, true));
        }
        sim.run(&b.ops()[n0..]);
        assert_eq!(sim.read(&index), x, "the index register is not restored");
        for &q in &index {
            sim.set(q, false);
        }
        sim.assert_clean();
        (got, t)
    }

    #[test]
    fn paired_lookup_is_exact_for_every_index() {
        let w = 5;
        for bits in 3..=7usize {
            for s in 1..bits {
                for (start, limit) in [
                    (0u64, 1u64 << bits),
                    (0, (1 << bits) - 3),
                    (1 << s, (1 << bits) - 1),
                    (3 << s, (5 << s).min((1 << bits) - 2)),
                ] {
                    if start >= limit {
                        continue;
                    }
                    let tab = table((bits * 17 + s) as u64 + limit, 1 << bits, w);
                    let bound = paired_cost(bits, start, limit, w, s);
                    let mut toff = None;
                    for x in 0..1u64 << bits {
                        for seed in [1u64, 2, 3] {
                            let parts = start > 0 || seed == 2;
                            let opt = Opts {
                                host: seed == 3,
                                collapse: seed != 1,
                                prune: false,
                            };
                            let (got, t) =
                                lane(bits, start, limit, w, s, &tab, x, seed + x, parts, opt);
                            let want = if (start..limit).contains(&x) {
                                tab[x as usize][0]
                            } else {
                                0
                            };
                            assert_eq!(got, want, "bits {bits} s {s} [{start},{limit}) x {x}");
                            // (The split expansion costs one AND more than the iteration's `2^s - 2`; its
                            // collapse replaces the fixup.)
                            let bound = bound + u64::from(opt.collapse);
                            assert!(t <= bound, "bits {bits} s {s}: {t} > {bound}");
                            if !opt.collapse {
                                assert!(toff.is_none_or(|u| u == t), "Toffolis depend on the lane");
                                toff = Some(t);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn paired_mutants_break_a_lane() {
        let (bits, s, w) = (6usize, 3usize, 5usize);
        let tab = table(77, 1 << bits, w);
        for fault in [40u8, 41, 42, 43, 44, 45] {
            let caught = (0..1u64 << bits).any(|x| {
                (1u64..4).any(|seed| {
                    std::panic::catch_unwind(|| {
                        FAULT.with(|c| c.set(fault));
                        let opt = Opts {
                            host: fault == 43 || fault == 44,
                            collapse: fault == 45,
                            prune: false,
                        };
                        let r = lane(bits, 0, 1 << bits, w, s, &tab, x, seed, false, opt);
                        FAULT.with(|c| c.set(0));
                        r
                    })
                    .map_or(true, |(got, _)| got != tab[x as usize][0])
                })
            });
            FAULT.with(|c| c.set(0));
            assert!(caught, "fault {fault} not caught");
        }
    }

    /// A table whose top slot subcubes repeat in every group (`runs` of them: the top
    /// `lam / 2^(j + 1)` values for `j < runs`, each made constant per group), as the padding
    /// buckets of lever `.p`.
    fn table_runs(seed: u64, bits: usize, s: usize, w: usize, runs: usize) -> Vec<Word> {
        let mut t = table(seed, 1 << bits, w);
        let lam = 1usize << s;
        for g in 0..1usize << (bits - s) {
            let mut top = lam;
            for j in 0..runs {
                let size = lam >> (j + 1);
                if size == 0 {
                    break;
                }
                let base = top - size;
                for v in base + 1..top {
                    t[g * lam + v] = t[g * lam + base].clone();
                }
                top = base;
            }
        }
        t
    }

    /// One lane of [`load_paired_pruned`] (the collapse, optional hosting), its erasure by
    /// `erase_parts`; returns the output, the Toffolis and the pruned leaf count.
    #[allow(clippy::too_many_arguments)]
    fn lane_pruned(
        bits: usize,
        start: u64,
        limit: u64,
        w: usize,
        s: usize,
        tab: &[Word],
        x: u64,
        seed: u64,
        host: bool,
    ) -> (u64, u64) {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let index = b.alloc_n(bits);
        let data = |y: u64| -> Word {
            if (start..limit).contains(&y) {
                tab[y as usize].clone()
            } else {
                word(w)
            }
        };
        let mut sim = Sim::new(&b, seed);
        for (i, &q) in index.iter().enumerate() {
            sim.set(q, x >> i & 1 == 1);
        }
        let opt = Opts {
            host,
            collapse: true,
            prune: true,
        };
        let r = load_paired_pruned(&mut b, &index, start, limit, w, &data, s, opt, None, None);
        sim.run(b.ops());
        let got = sim.read(&r.out);
        let t = sim.toffolis;
        let n0 = b.ops().len();
        let outb: Vec<_> = r.out.iter().map(|&q| b.hmr(q)).collect();
        let none = |_: u64| word(0);
        let split = best_split(bits, limit, true);
        qroam::erase_parts(
            &mut b,
            &index,
            r,
            &data,
            outb,
            &data,
            Vec::new(),
            &none,
            split,
        );
        sim.run(&b.ops()[n0..]);
        assert_eq!(sim.read(&index), x, "the index register is not restored");
        for &q in &index {
            sim.set(q, false);
        }
        sim.assert_clean();
        (got, t)
    }

    /// Lever `.p`: for every index value of small tables with repeated top subcubes (0 to 3 runs,
    /// ranges with odd ends, every slot width, with and without hosting), the pruned read leaves
    /// exactly `data(x)`, the erasure returns every ancilla clean, the Toffolis do not depend on
    /// the lane and equal [`paired_cost`] less the pruned slots (when every pair is full), and a
    /// run's slots are merged (fewer leaves than `2^s`).
    #[test]
    fn pruned_paired_lookup_is_exact_for_every_index() {
        let w = 4;
        for bits in 3..=7usize {
            for s in 1..bits {
                for runs in 0..=3usize {
                    for (start, limit) in [
                        (0u64, 1u64 << bits),
                        (0, (1 << bits) - 3),
                        (1 << s, (1 << bits) - 1),
                    ] {
                        if start >= limit {
                            continue;
                        }
                        let tab =
                            table_runs((bits * 31 + s * 7 + runs) as u64 + limit, bits, s, w, runs);
                        let data = |y: u64| -> Word {
                            if (start..limit).contains(&y) {
                                tab[y as usize].clone()
                            } else {
                                word(w)
                            }
                        };
                        let lam = 1u64 << s;
                        let (g0, g1) = (start / lam, limit.div_ceil(lam));
                        let merge = |base: u64, size: u64| {
                            (g0..g1).all(|g| {
                                (base..base + size).all(|v| {
                                    let x = g * lam + v;
                                    data(x) == data(g * lam + base)
                                })
                            })
                        };
                        let leaves = prune_leaves(s, &merge);
                        if runs > 0 && s >= 2 && limit == 1 << bits {
                            assert!(leaves.len() < 1 << s, "a run is merged");
                        }
                        for host in [false, true] {
                            let mut toff = None;
                            for x in 0..1u64 << bits {
                                let (got, t) =
                                    lane_pruned(bits, start, limit, w, s, &tab, x, 5 + x, host);
                                let want = if (start..limit).contains(&x) {
                                    tab[x as usize][0]
                                } else {
                                    0
                                };
                                assert_eq!(
                                    got, want,
                                    "bits {bits} s {s} runs {runs} [{start},{limit}) x {x}"
                                );
                                assert!(toff.is_none_or(|u| u == t), "Toffolis depend on the lane");
                                toff = Some(t);
                            }
                            let bound = paired_cost(bits, start, limit, w, s) + 1
                                - qroam::fixup_cost(s, lam, best_split(s, lam, true))
                                - (lam - leaves.len() as u64);
                            assert!(
                                toff.unwrap() <= bound,
                                "bits {bits} s {s} runs {runs}: {} > {bound}",
                                toff.unwrap()
                            );
                        }
                    }
                }
            }
        }
    }

    /// Lever `.p`'s mutants break a lane: a subcube merged although its entries differ (113), the
    /// leftover fan-out skipped (40), a hosted leaf not moved back (44), an index bit not rebuilt
    /// (43), a collapse `CZ` skipped (45).
    #[test]
    fn pruned_paired_mutants_break_a_lane() {
        let (bits, s, w) = (6usize, 3usize, 4usize);
        let tab = table_runs(91, bits, s, w, 2);
        for fault in [113u8, 40, 43, 44, 45] {
            let caught = (0..1u64 << bits).any(|x| {
                (1u64..4).any(|seed| {
                    std::panic::catch_unwind(|| {
                        FAULT.with(|c| c.set(fault));
                        let r = lane_pruned(bits, 0, 1 << bits, w, s, &tab, x, seed, true);
                        FAULT.with(|c| c.set(0));
                        r
                    })
                    .map_or(true, |(got, _)| got != tab[x as usize][0])
                })
            });
            FAULT.with(|c| c.set(0));
            assert!(caught, "fault {fault} not caught");
        }
    }
}
