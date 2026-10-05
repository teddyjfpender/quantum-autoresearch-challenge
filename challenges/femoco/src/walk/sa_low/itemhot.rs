//! Lever `H`: the inner alias read from a paired item one-hot.
//!
//! The outer item (the square `(r, c)`, `n = R C` items from `start`) is written into a split
//! one-hot of `G = 2` groups: `e1 = ceil(n / 2)` slots `h` and one group bit `g` (item `s` sits at
//! slot `s mod e1`, group `s / e1`). The inner word of `(item, i)` is then, for each inner index
//! `i`, a function of the one-hot, and the read iterates over `i` only:
//!
//! - at the leaf of `i` (flag `f`), `u1 = f AND g` (one Toffoli) and `u0 = f ^ u1` (in place);
//! - for every word bit `k`, `(u0 ^ X(h)) (u1 ^ Y(h)) = u0 Y ^ u1 X ^ X Y` with `Y` group 0's bit
//!   `k` and `X` group 1's (parities XORed into `u0`, `u1` in place): **one Toffoli per bit**
//!   gives `u0 L0 ^ u1 L1`, the bit of the lane's word, gated by the leaf (`u0 u1 = 0`);
//! - the left-over `X Y` is a function of the one-hot alone, applied on every leaf; its XOR over
//!   all `i` is cancelled once at the end by CNOT fan-out.
//!
//! The pairing is lever `z`'s (onehot.rs), applied to the two exclusive gated controls `u0, u1`.
//! The read costs `n_i (w + 1)` Toffolis plus the iteration over `i`, with no QROAM transient
//! beyond the one-hot. A lane whose item is outside the range (one-body) has an empty one-hot and
//! reads 0, as the item-layout table does.
//!
//! **Erasure** ([`phase`]): a phase `(-1)^(m . word(item, i))` is the same product with `CZ` in
//! place of the Toffoli target: per `i`, one Toffoli for `u1` and Cliffords for every bit (each
//! conditioned on its outcome), plus `Z` on the slots of the accumulated `X Y` at the end.
//!
//! **The one-hot itself** is written by an exact range iteration over the item index ([`write`],
//! `n` leaves) and erased by X measurement plus one fixup over the item index ([`erase`]); its
//! qubits stay allocated, so a copy can erase and rewrite it in place (the second copy replays the
//! same qubits).
use super::qroam::{self, Split};
use super::range;
use crate::circuit::{Bit, Builder, Qubit};
use crate::walk::common::lookup::hmr_keep;
use crate::walk::common::unary::{erase_and, iterate};
use crate::walk::shared::lookup::{word, Word};

/// A table over `(item, i)` (the inner word or an erasure's content).
pub type ItemData<'a> = dyn Fn(u64, u64) -> Word + 'a;

/// A measured part of an erasure: its outcomes and what the measured qubits held.
pub type Part<'a, 'b> = (&'a [Bit], &'b ItemData<'b>);

/// The paired item one-hot: `q[slot]`, group bit `g`, for items `start..start + n`.
///
/// **Aligned layout** (lever `V`): item `v` sits at slot `(v >> 1) - (start >> 1)`
/// with group `v & 1`, so the group bit is the item register's own low bit (`g` is that qubit,
/// never written or measured here) and the write iterates over the slot index `v >> 1` only,
/// about `n / 2` leaves instead of `n`. A boundary slot that holds only one in-range item is
/// written with one extra AND of the leaf flag and the (negated) group bit, so an out-of-range
/// lane's one-hot stays empty.
pub struct ItemHot {
    pub q: Vec<Qubit>,
    pub g: Qubit,
    pub e1: usize,
    pub start: u64,
    pub n: usize,
    pub aligned: bool,
    /// Lever `I` (aligned only): `Some(is_ob)` writes the one-hot by an in-place
    /// expansion of the slot index rooted at `NOT is_ob`, and erases it by the measured collapse
    /// ([`collapse`]): no phase fixup over the item index.
    pub inplace: Option<Qubit>,
    /// Aligned layouts: the number of low item bits that name the group (1 for lever `V`, 2 for
    /// lever `Y`'s four groups); 0 for the contiguous layout.
    pub shift: usize,
    /// Aligned layouts: the item register's low `shift` qubits (the group, never written here).
    pub glo: Vec<Qubit>,
}

impl ItemHot {
    /// Allocates the register (all `|0>`).
    pub fn alloc(b: &mut Builder, start: u64, n: usize) -> Self {
        let e1 = n.div_ceil(2);
        Self {
            q: b.alloc_n(e1),
            g: b.alloc(),
            e1,
            start,
            n,
            aligned: false,
            inplace: None,
            shift: 0,
            glo: Vec::new(),
        }
    }

    /// Allocates the aligned register (lever `V`): `g` is the item register's low bit.
    pub fn alloc_aligned(b: &mut Builder, start: u64, n: usize, g: Qubit) -> Self {
        assert!(n > 0, "an empty item range");
        let e1 = (((start + n as u64 - 1) >> 1) - (start >> 1) + 1) as usize;
        Self {
            q: b.alloc_n(e1),
            g,
            e1,
            start,
            n,
            aligned: true,
            inplace: None,
            shift: 1,
            glo: vec![g],
        }
    }

    /// Lever `Y`: the aligned register with `2^s` groups, `s = glo.len() >= 2`:
    /// item `v` at slot `(v >> s) - (start >> s)`, group `v mod 2^s` (the item register's own low
    /// `s` qubits `glo`). Written and erased in place only (lever `I`).
    pub fn alloc_aligned_groups(b: &mut Builder, start: u64, n: usize, glo: &[Qubit]) -> Self {
        let s = glo.len();
        assert!(
            n > 0 && s >= 2,
            "an empty item range or fewer than four groups"
        );
        let e1 = (((start + n as u64 - 1) >> s) - (start >> s) + 1) as usize;
        Self {
            q: b.alloc_n(e1),
            g: glo[0],
            e1,
            start,
            n,
            aligned: true,
            inplace: None,
            shift: s,
            glo: glo.to_vec(),
        }
    }

    /// The number of groups.
    #[must_use]
    pub fn groups(&self) -> usize {
        if self.aligned {
            1 << self.shift
        } else {
            2
        }
    }

    /// The in-place expansion's splits, in write order: `(node slot, child slot, index bit)`.
    /// The slot index is `item >> 1` over `item_bits - 1` bits; a node that covers values on both
    /// sides of its bit within the range splits (one AND), a one-sided node passes through.
    #[must_use]
    pub fn splits(&self, item_bits: usize) -> Vec<(usize, usize, usize)> {
        fn rec(
            out: &mut Vec<(usize, usize, usize)>,
            base: u64,
            k: usize,
            node: usize,
            sb: u64,
            se: u64,
        ) {
            if k == 0 {
                return;
            }
            let half = 1u64 << (k - 1);
            let left = base < se + 1 && base + half > sb;
            let right = base + half <= se && base + 2 * half > sb;
            match (left, right) {
                (true, true) => {
                    let child = (base + half - sb) as usize;
                    out.push((node, child, k - 1));
                    rec(out, base, k - 1, node, sb, se);
                    rec(out, base + half, k - 1, child, sb, se);
                }
                (true, false) => rec(out, base, k - 1, node, sb, se),
                (false, true) => rec(out, base + half, k - 1, node, sb, se),
                (false, false) => {}
            }
        }
        assert!(
            self.aligned,
            "the in-place expansion is for the aligned layout"
        );
        let sb = self.start >> self.shift;
        let se = sb + self.e1 as u64 - 1;
        let mut out = Vec::new();
        rec(&mut out, 0, item_bits - self.shift, 0, sb, se);
        out
    }

    /// Whether item `v` is in the register's range.
    #[must_use]
    pub fn in_range(&self, v: u64) -> bool {
        v >= self.start && v < self.start + self.n as u64
    }

    /// The `(slot, group)` of in-range item `v`.
    #[must_use]
    pub fn place(&self, v: u64) -> (usize, usize) {
        if self.aligned {
            let s = self.shift;
            (
                ((v >> s) - (self.start >> s)) as usize,
                (v & ((1 << s) - 1)) as usize,
            )
        } else {
            let s = (v - self.start) as usize;
            (s % self.e1, s / self.e1)
        }
    }

    /// The item at `group`, `slot` (`None`: no item).
    #[must_use]
    pub fn item(&self, group: usize, slot: usize) -> Option<u64> {
        if self.aligned {
            let s = self.shift;
            if group >> s != 0 {
                return None;
            }
            let v = ((self.start >> s) + slot as u64) << s | group as u64;
            return self.in_range(v).then_some(v);
        }
        let s = group * self.e1 + slot;
        (s < self.n).then(|| self.start + s as u64)
    }
}

/// Writes the lane's item into `hot` (which must be `|0>`): an exact iteration over `item` for
/// `start..start + n`.
pub fn write(b: &mut Builder, hot: &ItemHot, item: &[Qubit]) {
    if let Some(ob) = hot.inplace {
        expand(b, hot, item, ob);
        return;
    }
    if hot.aligned {
        assert_eq!(hot.shift, 1, "lever Y is written in place only (I)");
        write_aligned(b, hot, item);
        return;
    }
    let (q, g, e1, start) = (&hot.q, hot.g, hot.e1, hot.start);
    range::iterate_range(
        b,
        None,
        item,
        start,
        start + hot.n as u64,
        &mut |b: &mut Builder, v: u64, f: Qubit| {
            let s = (v - start) as usize;
            b.cx(f, q[s % e1]);
            if s >= e1 && !super::onehot::fault_is(30) {
                b.cx(f, g);
            }
        },
    );
}

/// [`write`] for the aligned layout: an iteration over the slot index (`item[1..]`); a boundary
/// slot with one in-range item is set by `flag AND g` (or `flag AND NOT g`) instead of `flag`.
fn write_aligned(b: &mut Builder, hot: &ItemHot, item: &[Qubit]) {
    assert_eq!(
        item[0], hot.g,
        "the aligned group bit is the item's low bit"
    );
    let sb = hot.start >> 1;
    let (q, g) = (&hot.q, hot.g);
    range::iterate_range(
        b,
        None,
        &item[1..],
        sb,
        sb + hot.e1 as u64,
        &mut |b: &mut Builder, v: u64, f: Qubit| {
            let slot = (v - sb) as usize;
            let lo = hot.in_range(v << 1);
            let hi = hot.in_range(v << 1 | 1);
            if super::onehot::fault_is(35) || (lo && hi) {
                b.cx(f, q[slot]);
            } else {
                // One in-range item: hot = flag AND [g = group of that item].
                if lo {
                    b.x(g);
                }
                b.ccx(f, g, q[slot]);
                if lo {
                    b.x(g);
                }
            }
        },
    );
}

/// Lever `I`: the in-place expansion. Slot 0 becomes `NOT is_ob` (1 exactly on the in-range
/// lanes: every item at or past `start` is a square), then each split moves the node's lanes
/// whose index bit is 1 into the child: `child = node AND u_k`, `node ^= child`. One Toffoli per
/// split (`e1 - 1`); the index register is read, never changed.
fn expand(b: &mut Builder, hot: &ItemHot, item: &[Qubit], is_ob: Qubit) {
    assert_eq!(
        item[0], hot.g,
        "the aligned group bit is the item's low bit"
    );
    let q0 = hot.q[0];
    b.cx(is_ob, q0);
    b.x(q0);
    for (node, child, k) in hot.splits(item.len()) {
        b.ccx(hot.q[node], item[hot.shift + k], hot.q[child]);
        b.cx(hot.q[child], hot.q[node]);
    }
}

/// Lever `I`: the measured collapse, the expansion run backwards with every AND erased by
/// measurement. A child is `node AND u_k` of two qubits that are still there, so an outcome of
/// 1 is cancelled by `CZ(node, u_k)`: Cliffords only, and no lookup over the item index. The
/// splits are undone level by level (index bit 0 first; splits at one bit commute), so the
/// one-hot shrinks as it goes. With `restore`, the slot index bits are cleared on the in-range
/// lanes (lever `K`) and each `u_k` is rebuilt from the surviving nodes just before its level is
/// undone, so the index and the full one-hot never coexist. The root, `NOT is_ob`, is cleared
/// from `is_ob`, which must be intact.
pub fn collapse(b: &mut Builder, hot: &ItemHot, item: &[Qubit], is_ob: Qubit, restore: bool) {
    let mut splits = hot.splits(item.len());
    // Stable sort by bit: within a bit the reverse of write order (any order is exact).
    splits.reverse();
    splits.sort_by_key(|s| s.2);
    let sh = hot.shift;
    let sb = hot.start >> sh;
    let mut alive = vec![true; hot.e1];
    let mut at = 0;
    for k in 0..item.len() - sh {
        if restore {
            // The surviving node at slot c covers the values from sb + c with the same bits above
            // the levels already undone, so bit k is the bit of sb + c.
            for (c, &q) in hot.q.iter().enumerate() {
                if alive[c] && (sb + c as u64) >> k & 1 == 1 {
                    b.cx(q, item[sh + k]);
                }
            }
        }
        while at < splits.len() && splits[at].2 == k {
            let (node, child, _) = splits[at];
            b.cx(hot.q[child], hot.q[node]);
            let m = hmr_keep(b, hot.q[child]);
            alive[child] = false;
            if !super::onehot::fault_is(36) {
                let ctl = if super::onehot::fault_is(37) {
                    k ^ 1
                } else {
                    k
                };
                if sh + ctl < item.len() {
                    b.cz_if(hot.q[node], item[sh + ctl], m);
                }
            }
            at += 1;
        }
    }
    let q0 = hot.q[0];
    b.x(q0);
    b.cx(is_ob, q0);
    super::onehot::reset_keep(b, q0);
}

/// Erases `hot` by X measurement (the qubits stay allocated) and one phase fixup over `item`
/// (`limit` values, split as `split`).
pub fn erase(b: &mut Builder, hot: &ItemHot, item: &[Qubit], limit: u64, split: Split) {
    if let Some(ob) = hot.inplace {
        collapse(b, hot, item, ob, false);
        return;
    }
    assert!(hot.shift <= 1, "lever Y is erased by the collapse only (I)");
    // The aligned layout's group bit is the item register's own qubit: not measured here.
    let own_g: &[Qubit] = if hot.aligned {
        &[]
    } else {
        std::slice::from_ref(&hot.g)
    };
    let m: Vec<Bit> = hot.q.iter().chain(own_g).map(|&q| hmr_keep(b, q)).collect();
    if super::onehot::fault_is(31) {
        return;
    }
    let bits = m.len();
    let data = |v: u64| -> Word {
        let mut w = word(bits);
        if hot.in_range(v) {
            let (slot, group) = hot.place(v);
            w[slot / 64] |= 1 << (slot % 64);
            if group > 0 && !hot.aligned {
                w[hot.e1 / 64] |= 1 << (hot.e1 % 64);
            }
        }
        w
    };
    qroam::fixup(b, item, limit, &m, &data, split);
    for &q in hot.q.iter().chain(own_g) {
        super::onehot::reset_keep(b, q);
    }
}

/// XORs `value(item)` into `targets` (bit `k` into `targets[k]`) for the lane's item, from the
/// one-hot: CNOTs from each slot for its group-0 item, and one Toffoli `(g, parity, target)` per
/// bit whose group difference is not zero. Nothing on a lane whose one-hot is empty.
pub fn fan(b: &mut Builder, hot: &ItemHot, targets: &[Qubit], value: &dyn Fn(u64) -> u64) {
    if hot.shift >= 2 {
        fan_groups(b, hot, targets, value);
        return;
    }
    let at = |g: usize, c: usize| hot.item(g, c).map_or(0, value);
    for (c, &h) in hot.q.iter().enumerate() {
        let v = at(0, c);
        for (k, &t) in targets.iter().enumerate() {
            if v >> k & 1 == 1 {
                b.cx(h, t);
            }
        }
    }
    for (k, &t) in targets.iter().enumerate() {
        let set: Vec<Qubit> = (0..hot.e1)
            .filter(|&c| hot.item(1, c).is_some() && (at(0, c) ^ at(1, c)) >> k & 1 == 1)
            .map(|c| hot.q[c])
            .collect();
        match set.as_slice() {
            [] => {}
            [h] => b.ccx(hot.g, *h, t),
            _ => {
                let s = b.alloc();
                set.iter().for_each(|&h| b.cx(h, s));
                b.ccx(hot.g, s, t);
                set.iter().for_each(|&h| b.cx(h, s));
                b.free(s);
            }
        }
    }
}

/// An item function for [`fan_rooted`]: a word per item.
pub type ItemFn<'a> = dyn Fn(u64) -> Word + 'a;

/// A conditioned phase part for [`fan_rooted`]: outcomes and the item function they weight.
pub type ItemPart<'a, 'b> = (&'a [Bit], &'b ItemFn<'b>);

/// Lever `+` (`padalias.rs`): under `root` (the walk control), XORs
/// `value(item)` into `targets` and applies `(-1)^(phase(item) ^ sum_p m_p . part_p(item))`, from
/// the two-group item one-hot. `c1 = root AND g` (one AND, erased by measurement) and
/// `c0 = root ^ c1` (CNOTs) are exclusive, so each target bit is one paired Toffoli
/// `(c0 ^ X(h)) (c1 ^ Y(h)) = c0 Y ^ c1 X ^ X Y` (`Y` group 0's support, `X` group 1's; the left-over
/// `X Y`, linear on the one-hot, removed by CNOTs), and every phase is `CZ`s of `c0` / `c1` with
/// slots (Cliffords, conditioned on the outcome for a part). Nothing where `root` is 0 or the
/// one-hot is empty. `1 + (bits with a nonzero support)` Toffolis.
///
/// # Panics
/// On the four-group item one-hot (lever `Y`).
pub fn fan_rooted(
    b: &mut Builder,
    hot: &ItemHot,
    root: Qubit,
    targets: &[Qubit],
    value: &ItemFn<'_>,
    phase: Option<&dyn Fn(u64) -> bool>,
    parts: &[ItemPart<'_, '_>],
) {
    assert!(
        hot.shift <= 1,
        "lever + is built for the two-group item one-hot"
    );
    let sup = |g: usize, f: &dyn Fn(u64) -> bool| -> Vec<usize> {
        (0..hot.e1)
            .filter(|&c| hot.item(g, c).is_some_and(f))
            .collect()
    };
    let c1 = b.alloc();
    b.ccx(root, hot.g, c1);
    let c0 = b.alloc();
    b.cx(root, c0);
    b.cx(c1, c0);
    for (k, &t) in targets.iter().enumerate() {
        let bit = |it: u64| value(it)[k / 64] >> (k % 64) & 1 == 1;
        let y = sup(0, &bit);
        let x = sup(1, &bit);
        if x.is_empty() && y.is_empty() {
            continue;
        }
        x.iter().for_each(|&c| b.cx(hot.q[c], c0));
        y.iter().for_each(|&c| b.cx(hot.q[c], c1));
        if !super::onehot::fault_is(100) {
            b.ccx(c0, c1, t);
        }
        x.iter().for_each(|&c| b.cx(hot.q[c], c0));
        y.iter().for_each(|&c| b.cx(hot.q[c], c1));
        for &c in &x {
            if y.contains(&c) && !super::onehot::fault_is(101) {
                b.cx(hot.q[c], t);
            }
        }
    }
    let cz_sup = |b: &mut Builder, f: &dyn Fn(u64) -> bool| {
        for c in sup(0, f) {
            b.cz(c0, hot.q[c]);
        }
        for c in sup(1, f) {
            b.cz(c1, hot.q[c]);
        }
    };
    if let Some(f) = phase {
        if !super::onehot::fault_is(102) {
            cz_sup(b, f);
        }
    }
    for (m, data) in parts {
        for (k, &mk) in m.iter().enumerate() {
            let bit = |it: u64| data(it)[k / 64] >> (k % 64) & 1 == 1;
            if sup(0, &bit).is_empty() && sup(1, &bit).is_empty() {
                continue;
            }
            b.push_condition(mk);
            if !super::onehot::fault_is(103) {
                cz_sup(b, &bit);
            }
            b.pop_condition();
        }
    }
    b.cx(c1, c0);
    b.cx(root, c0);
    b.free(c0);
    erase_and(b, root, hot.g, c1, false);
}

/// Lever `+`: the number of inner indices a read (or phase pass) over `data` must iterate:
/// one past the last `i` whose word is nonzero for some item of the one-hot.
#[must_use]
pub fn rows_needed(hot: &ItemHot, n_i: u64, data: &[&dyn Fn(u64, u64) -> Word]) -> u64 {
    let items: Vec<u64> = (0..hot.groups())
        .flat_map(|g| (0..hot.e1).filter_map(move |c| hot.item(g, c)))
        .collect();
    (0..n_i)
        .rev()
        .find(|&i| {
            items
                .iter()
                .any(|&it| data.iter().any(|d| d(it, i).iter().any(|&x| x != 0)))
        })
        .map_or(0, |i| i + 1)
}

/// The slots whose item (in `group`) has bit `k` of `data(item, i)` set.
fn support(
    hot: &ItemHot,
    group: usize,
    i: u64,
    k: usize,
    data: &dyn Fn(u64, u64) -> Word,
) -> Vec<usize> {
    (0..hot.e1)
        .filter(|&c| {
            hot.item(group, c)
                .is_some_and(|it| data(it, i)[k / 64] >> (k % 64) & 1 == 1)
        })
        .collect()
}

/// Reads `w` bits of `data(item, i)` into a fresh register, iterating over the inner index
/// `i_reg` (`n_i` values). One Toffoli per `i` for the gated group control and one per bit.
pub fn read(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    w: usize,
    data: &dyn Fn(u64, u64) -> Word,
) -> Vec<Qubit> {
    let out = b.alloc_n(w);
    read_into(b, hot, i_reg, n_i, &out, data);
    out
}

/// [`read`] XORing into a caller's register `out` (`w = out.len()` bits); on a lane whose one-hot
/// is empty `out` is left unchanged (lever `D` keeps other data there on those lanes).
pub fn read_into(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    out: &[Qubit],
    data: &dyn Fn(u64, u64) -> Word,
) {
    read_into_ext(b, hot, i_reg, n_i, out, data, None, None);
}

/// [`read_into`] with two options (lever `t`; both `None` emits exactly
/// [`read_into`]'s ops):
///
/// - `root`: the iteration over `i` is rooted at this qubit (the walk's control), so every leaf
///   flag, and with it everything read and every phase applied, is gated by it: on a lane where
///   it is 0 the register stays 0 and no phase is applied;
/// - `phase`: a lane phase `(-1)^(phase(item, i))` applied in the same pass at no Toffoli cost:
///   at each leaf the two in-place parities of its group-0 and group-1 slots are joined by a
///   `CZ` of the gated controls (`(u0 ^ X)(u1 ^ Y) = u0 Y ^ u1 X ^ X Y`), and the left-over
///   `X Y` (a function of the one-hot alone, applied on every lane) is cancelled once at the end
///   by `Z` on its slots.
#[allow(clippy::too_many_arguments)]
pub fn read_into_ext(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    out: &[Qubit],
    data: &dyn Fn(u64, u64) -> Word,
    root: Option<Qubit>,
    phase: Option<&dyn Fn(u64, u64) -> bool>,
) {
    if hot.shift >= 2 {
        assert!(
            root.is_none() && phase.is_none(),
            "lever t's rooted read is not built for the four-group item one-hot (T)"
        );
        read_groups(b, hot, i_reg, n_i, out, data);
        return;
    }
    let w = out.len();
    let mut xy = vec![vec![false; hot.e1]; w];
    let mut xy_phase = vec![false; hot.e1];
    let pdata = |it: u64, i: u64| -> Word {
        let mut v = word(1);
        if phase.is_some_and(|f| f(it, i)) {
            v[0] = 1;
        }
        v
    };
    iterate(
        b,
        root,
        i_reg,
        n_i,
        &mut |b: &mut Builder, i: u64, f: Qubit| {
            let u1 = b.alloc();
            b.ccx(f, hot.g, u1);
            b.cx(u1, f);
            if phase.is_some() {
                let y = support(hot, 0, i, 0, &pdata);
                let x = support(hot, 1, i, 0, &pdata);
                if !(x.is_empty() && y.is_empty()) {
                    x.iter().for_each(|&c| b.cx(hot.q[c], f));
                    y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                    if !super::onehot::fault_is(90) {
                        b.cz(f, u1);
                    }
                    x.iter().for_each(|&c| b.cx(hot.q[c], f));
                    y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                    for &c in &x {
                        if y.contains(&c) {
                            xy_phase[c] ^= true;
                        }
                    }
                }
            }
            for (k, &o) in out.iter().enumerate() {
                let y = support(hot, 0, i, k, data);
                let x = support(hot, 1, i, k, data);
                if x.is_empty() && y.is_empty() {
                    continue;
                }
                x.iter().for_each(|&c| b.cx(hot.q[c], f));
                y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                b.ccx(f, u1, o);
                x.iter().for_each(|&c| b.cx(hot.q[c], f));
                y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                if !super::onehot::fault_is(32) {
                    for &c in &x {
                        if y.contains(&c) {
                            xy[k][c] ^= true;
                        }
                    }
                }
            }
            b.cx(u1, f);
            erase_and(b, f, hot.g, u1, false);
        },
    );
    for (k, &o) in out.iter().enumerate() {
        for (c, &on) in xy[k].iter().enumerate() {
            if on {
                b.cx(hot.q[c], o);
            }
        }
    }
    if !super::onehot::fault_is(91) {
        for (c, &on) in xy_phase.iter().enumerate() {
            if on {
                b.z(hot.q[c]);
            }
        }
    }
}

/// Applies `(-1)^(sum_j m_j . data_j(item, i))` for every part `(m_j, data_j)`: per `i`, one
/// Toffoli for the gated group control and, for every outcome bit, a `CZ` of the two in-place
/// parities conditioned on it; the accumulated `X Y` slots get conditioned `Z`s at the end.
pub fn phase(b: &mut Builder, hot: &ItemHot, i_reg: &[Qubit], n_i: u64, parts: &[Part<'_, '_>]) {
    phase_rooted(b, hot, i_reg, n_i, parts, None);
}

/// [`phase`] with the iteration over `i` rooted at `root` (lever `t`): the phases are applied
/// only where `root` is 1 (the left-over slot phases, functions of the one-hot alone, cancel on
/// every lane as before). `None` emits exactly [`phase`]'s ops.
pub fn phase_rooted(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    parts: &[Part<'_, '_>],
    root: Option<Qubit>,
) {
    if hot.shift >= 2 {
        assert!(
            root.is_none(),
            "lever t's rooted phase is not built for the four-group item one-hot (T)"
        );
        phase_groups(b, hot, i_reg, n_i, parts);
        return;
    }
    let mut xy: Vec<Vec<Vec<bool>>> = parts
        .iter()
        .map(|(m, _)| vec![vec![false; hot.e1]; m.len()])
        .collect();
    iterate(
        b,
        root,
        i_reg,
        n_i,
        &mut |b: &mut Builder, i: u64, f: Qubit| {
            let u1 = b.alloc();
            b.ccx(f, hot.g, u1);
            b.cx(u1, f);
            for (p, (m, data)) in parts.iter().enumerate() {
                for (k, &mk) in m.iter().enumerate() {
                    let y = support(hot, 0, i, k, *data);
                    let x = support(hot, 1, i, k, *data);
                    if x.is_empty() && y.is_empty() {
                        continue;
                    }
                    b.push_condition(mk);
                    x.iter().for_each(|&c| b.cx(hot.q[c], f));
                    y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                    b.cz(f, u1);
                    x.iter().for_each(|&c| b.cx(hot.q[c], f));
                    y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                    b.pop_condition();
                    for &c in &x {
                        if y.contains(&c) {
                            xy[p][k][c] ^= true;
                        }
                    }
                }
            }
            b.cx(u1, f);
            erase_and(b, f, hot.g, u1, false);
        },
    );
    if super::onehot::fault_is(33) {
        return;
    }
    for (p, (m, _)) in parts.iter().enumerate() {
        for (k, &mk) in m.iter().enumerate() {
            let slots: Vec<usize> = (0..hot.e1).filter(|&c| xy[p][k][c]).collect();
            if slots.is_empty() {
                continue;
            }
            b.push_condition(mk);
            for c in slots {
                b.z(hot.q[c]);
            }
            b.pop_condition();
        }
    }
}

/// Lever `~`: erases `lt = [draw < keep(item, i)]` after the inner keep register was
/// X-measured at PREPARE, by the outcome-gated re-read (`narrow::gated_lt_erase_m`,
/// lever `j`) applied to the item one-hot. `m` is `lt`'s X outcome (the caller
/// measured `lt` already; `(-1)^(m lt)` is what is left to cancel). Only where `m = 1`:
/// `keep(item, i)` is read again from the one-hot into a fresh `mu`-qubit register
/// ([`read_pooled`]: the read of [`read_into_ext`] / [`read_groups`] with every AND from and back to
/// a pool), `(-1)^[draw < keep]` is applied (`narrow::lt_phase`: `mu - 1` carries, erased by
/// measurement), and the register is X-measured. Returns those outcomes (0 where the block did not
/// run): their content is `keep(item, i)` on the `m = 1` lanes, so the caller's erasure pass
/// cancels them as one more part with the same data (Cliffords).
///
/// Expected Toffolis: half of `read + mu - 1`, with `read` the item one-hot read of `n_i` rows.
/// `root` gates the read as lever `t`'s rooted read is gated (the keep is 0 where it is 0).
#[allow(clippy::too_many_arguments)]
pub fn gated_lt_erase_hot(
    b: &mut Builder,
    hot: &ItemHot,
    m: Bit,
    draw: &[Qubit],
    i_reg: &[Qubit],
    n_i: u64,
    keep: &ItemData<'_>,
    root: Option<Qubit>,
    fault: super::narrow::Fault,
) -> Vec<Bit> {
    use super::narrow::{self, Fault, Pool};
    use crate::circuit::{Op, OperationType};
    let mu = draw.len();
    let bits: Vec<Bit> = (0..mu).map(|_| narrow::zero_bit(b)).collect();
    let flip = |b: &mut Builder| {
        let mut op = Op::new(OperationType::BitInvert);
        op.c_target = m.0;
        b.emit(op);
    };
    if fault == Fault::FlipOutcome {
        flip(b);
    }
    let mut pool = Pool::default();
    b.push_condition(m);
    let out = pool.take_n(b, mu);
    if !super::onehot::fault_is(203) {
        read_pooled(b, hot, i_reg, n_i, &out, keep, root, &mut pool);
    }
    if fault != Fault::NoPhase {
        // The in-place comparator: one scratch qubit, so the block's footprint is the read's.
        narrow::lt_phase_inplace(b, draw, &out, &mut pool, fault != Fault::NoTopZ);
    }
    for (&q, &bit) in out.iter().zip(&bits) {
        pool.measure_into(b, q, bit);
    }
    b.pop_condition();
    if fault == Fault::FlipOutcome {
        flip(b);
    }
    pool.close(b);
    bits
}

/// [`read_into_ext`] (no phase) and [`read_groups`] inside a condition block: XORs `data(item, i)`
/// into `out` over `n_i` inner indices, every AND (the iteration's, the gated group controls')
/// taken from and measured back into `pool`. `G / 2` Toffolis per word bit per `i` (`G` the
/// one-hot's groups) and about `G n_i` for the iteration and the gated controls.
#[allow(clippy::too_many_arguments)]
pub fn read_pooled(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    out: &[Qubit],
    data: &ItemData<'_>,
    root: Option<Qubit>,
    pool: &mut super::narrow::Pool,
) {
    use super::narrow::{self, Pool};
    if n_i == 0 {
        return;
    }
    // The group bits: the item register's own low bits (aligned), or the separate `g`.
    let glo: Vec<Qubit> = if hot.glo.is_empty() {
        vec![hot.g]
    } else {
        hot.glo.clone()
    };
    // With `2^s` groups the group bits above the lowest join the iteration as its low bits, so a
    // leaf is `(i, pair)` and only the pair's two gated controls are made: the same Toffolis as
    // splitting every leaf of `i` into `2^s` controls (`2^s n_i - 2` in all), with
    // `log2(2^s n_i) - 1` ANDs live at once instead of `log2(n_i) - 1 + 2^s - 1`.
    let s = glo.len();
    let idx: Vec<Qubit> = glo[1..].iter().chain(i_reg).copied().collect();
    let pmask = (1u64 << (s - 1)) - 1;
    let w = out.len();
    let mut xy = vec![vec![false; hot.e1]; w];
    let mut inner = Pool::default();
    narrow::iterate(
        b,
        root,
        &idx,
        n_i << (s - 1),
        &mut |b: &mut Builder, x: u64, f: Qubit| {
            let (p, i) = ((x & pmask) as usize, x >> (s - 1));
            let (ga, gb) = (2 * p, 2 * p + 1);
            // u1 = f AND g0, u0 = f ^ u1 (in place).
            let u1 = inner.take(b);
            b.ccx(f, glo[0], u1);
            b.cx(u1, f);
            for (k, &o) in out.iter().enumerate() {
                let y = support(hot, ga, i, k, data);
                let x = support(hot, gb, i, k, data);
                if x.is_empty() && y.is_empty() {
                    continue;
                }
                x.iter().for_each(|&c| b.cx(hot.q[c], f));
                y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                b.ccx(f, u1, o);
                x.iter().for_each(|&c| b.cx(hot.q[c], f));
                y.iter().for_each(|&c| b.cx(hot.q[c], u1));
                if !super::onehot::fault_is(204) {
                    for &c in &x {
                        if y.contains(&c) {
                            xy[k][c] ^= true;
                        }
                    }
                }
            }
            b.cx(u1, f);
            let mm = inner.measure(b, u1);
            b.cz_if(f, glo[0], mm);
        },
        pool,
    );
    pool.absorb(inner);
    for (k, &o) in out.iter().enumerate() {
        for (c, &on) in xy[k].iter().enumerate() {
            if on {
                b.cx(hot.q[c], o);
            }
        }
    }
}

/// Lever `.e`: [`phase_rooted`] with the inner iteration's sibling leaves paired. Per
/// leaf `f` the phase is `(-1)^(f P0 ^ (f AND g) D)` with `P0` group 0's parity and `D = P0 ^ P1`
/// the group difference (functions of the one-hot, linear in the outcome bits). `f P0` takes
/// conditioned `CZ`s (Cliffords). For two siblings `l`, `r` (never both 1) the group terms come
/// from one `CCZ(g, l ^ D_r, r ^ D_l) = (-1)^(g (l D_l ^ r D_r ^ D_l D_r))` (the parities XORed into
/// the flags in place and undone) instead of one AND per leaf: a Toffoli per pair, not per leaf.
/// The left-over `g D_l D_r` is, on the one-hot, `g` times the parity of the slots in both
/// supports; its coefficient per slot is quadratic in the outcome bits, so it is accumulated in
/// one classical bit per slot (`T_c ^= D_l(c) D_r(c)`, computed by conditioned bit inversions) and
/// applied once at the end as `CZ(g, h_c)` conditioned on `T_c`. A lone leaf takes
/// `CCZ(g, f, s)` with `s` its difference parity gathered in a scratch qubit. Only the two-group
/// one-hot (`shift <= 1`); supports are used directly (no complements), so an empty one-hot
/// (one-body lanes) gets no phase, as in [`phase_rooted`].
pub fn phase_rooted_paired(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    parts: &[Part<'_, '_>],
    root: Option<Qubit>,
) {
    assert!(hot.shift <= 1, "lever .e pairs the two-group item one-hot");
    let e1 = hot.e1;
    // Per leaf `i`: per part `p`, per outcome bit `k`: (group-0 support, difference support).
    let sets = |i: u64| -> Vec<Vec<(Vec<usize>, Vec<bool>)>> {
        parts
            .iter()
            .map(|(m, data)| {
                (0..m.len())
                    .map(|k| {
                        let y = support(hot, 0, i, k, *data);
                        let x = support(hot, 1, i, k, *data);
                        let mut d = vec![false; e1];
                        for &c in &y {
                            d[c] ^= true;
                        }
                        for &c in &x {
                            d[c] ^= true;
                        }
                        (y, d)
                    })
                    .collect()
            })
            .collect()
    };
    let tot: Vec<crate::circuit::Bit> = (0..e1).map(|_| super::narrow::zero_bit(b)).collect();
    let mut tot_used = vec![false; e1];
    // XORs the (outcome-weighted) difference parity of leaf `sx` into `q` (call twice to undo).
    let xor_diff = |b: &mut Builder, sx: &[Vec<(Vec<usize>, Vec<bool>)>], q: Qubit| {
        for (p, (m, _)) in parts.iter().enumerate() {
            for (k, &mk) in m.iter().enumerate() {
                let d = &sx[p][k].1;
                if !d.iter().any(|&x| x) {
                    continue;
                }
                b.push_condition(mk);
                for (c, &on) in d.iter().enumerate() {
                    if on {
                        b.cx(hot.q[c], q);
                    }
                }
                b.pop_condition();
            }
        }
    };
    let p0 = |b: &mut Builder, sx: &[Vec<(Vec<usize>, Vec<bool>)>], f: Qubit| {
        for (p, (m, _)) in parts.iter().enumerate() {
            for (k, &mk) in m.iter().enumerate() {
                let y = &sx[p][k].0;
                if y.is_empty() {
                    continue;
                }
                b.push_condition(mk);
                for &c in y {
                    b.cz(f, hot.q[c]);
                }
                b.pop_condition();
            }
        }
    };
    let any_diff = |sx: &[Vec<(Vec<usize>, Vec<bool>)>]| {
        sx.iter()
            .any(|pp| pp.iter().any(|(_, d)| d.iter().any(|&x| x)))
    };
    let g = hot.g;
    {
        let mut pair = |b: &mut Builder, base: u64, l: Option<Qubit>, r: Option<Qubit>| {
            let sl = l.map(|_| sets(base));
            let sr = r.map(|_| sets(base + 1));
            if let (Some(f), Some(sx)) = (l, &sl) {
                p0(b, sx, f);
            }
            if let (Some(f), Some(sx)) = (r, &sr) {
                p0(b, sx, f);
            }
            match (l, r) {
                (Some(lq), Some(rq)) => {
                    let (sl, sr) = (sl.as_ref().unwrap(), sr.as_ref().unwrap());
                    if !any_diff(sl) && !any_diff(sr) {
                        return;
                    }
                    xor_diff(b, sr, lq);
                    xor_diff(b, sl, rq);
                    if !fault_is_e(130) {
                        b.ccz(g, lq, rq);
                    }
                    xor_diff(b, sr, lq);
                    xor_diff(b, sl, rq);
                    // T_c ^= D_l(c) D_r(c), each a parity of outcome bits.
                    for c in 0..e1 {
                        let terms =
                            |sx: &[Vec<(Vec<usize>, Vec<bool>)>]| -> Vec<crate::circuit::Bit> {
                                let mut v = Vec::new();
                                for (p, (m, _)) in parts.iter().enumerate() {
                                    for (k, &mk) in m.iter().enumerate() {
                                        if sx[p][k].1[c] {
                                            v.push(mk);
                                        }
                                    }
                                }
                                v
                            };
                        let (tl, tr) = (terms(sl), terms(sr));
                        if tl.is_empty() || tr.is_empty() || fault_is_e(131) {
                            continue;
                        }
                        let parity = |b: &mut Builder, t: &[crate::circuit::Bit]| {
                            let x = super::narrow::zero_bit(b);
                            for &mk in t {
                                b.push_condition(mk);
                                let mut op = crate::circuit::Op::new(
                                    crate::circuit::OperationType::BitInvert,
                                );
                                op.c_target = x.0;
                                b.emit(op);
                                b.pop_condition();
                            }
                            x
                        };
                        let (xl, xr) = (parity(b, &tl), parity(b, &tr));
                        b.push_condition(xl);
                        b.push_condition(xr);
                        let mut op =
                            crate::circuit::Op::new(crate::circuit::OperationType::BitInvert);
                        op.c_target = tot[c].0;
                        b.emit(op);
                        b.pop_condition();
                        b.pop_condition();
                        tot_used[c] = true;
                    }
                }
                (Some(f), None) | (None, Some(f)) => {
                    let sx = if l.is_some() {
                        sl.as_ref()
                    } else {
                        sr.as_ref()
                    }
                    .unwrap();
                    if !any_diff(sx) {
                        return;
                    }
                    let s = b.alloc();
                    xor_diff(b, sx, s);
                    b.ccz(g, f, s);
                    xor_diff(b, sx, s);
                    b.free(s);
                }
                (None, None) => {}
            }
        };
        super::paired::iterate_pairs(b, root, i_reg, 0, n_i, &mut pair);
    }
    if fault_is_e(132) {
        return;
    }
    for c in 0..e1 {
        if tot_used[c] {
            b.push_condition(tot[c]);
            b.cz(g, hot.q[c]);
            b.pop_condition();
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static FAULT_E: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

fn fault_is_e(k: u8) -> bool {
    #[cfg(test)]
    {
        FAULT_E.with(std::cell::Cell::get) == k || super::onehot::fault_is(k)
    }
    #[cfg(not(test))]
    {
        let _ = k;
        false
    }
}

/// Lever `Y`: the gated group controls of a leaf flag `f`, `u_g = f AND [glo = g]` for the `2^s`
/// groups, by an in-place expansion of `f` over the group bits (`2^s - 1` ANDs; `u_0` is `f`'s
/// own qubit). Exclusive, and all 0 where `f` is.
fn gate_split(b: &mut Builder, f: Qubit, glo: &[Qubit]) -> Vec<Qubit> {
    let mut nodes = vec![f];
    for (j, &x) in glo.iter().enumerate() {
        for idx in 0..1usize << j {
            let child = b.alloc();
            b.ccx(nodes[idx], x, child);
            b.cx(child, nodes[idx]);
            nodes.push(child);
        }
    }
    nodes
}

/// Undoes [`gate_split`]: each AND erased by measurement (its phase a `CZ`).
fn gate_merge(b: &mut Builder, nodes: &[Qubit], glo: &[Qubit]) {
    for j in (0..glo.len()).rev() {
        for idx in (0..1usize << j).rev() {
            let (node, child) = (nodes[idx], nodes[idx | 1 << j]);
            b.cx(child, node);
            erase_and(b, node, glo[j], child, false);
        }
    }
}

/// Lever `Y`'s read: per `i`, the gated controls `u_g`, then for every word bit one Toffoli per
/// group pair `(2p, 2p + 1)`: `(u_2p ^ X(h)) (u_2p+1 ^ Y(h)) = u_2p Y ^ u_2p+1 X ^ X Y` with `Y`
/// group `2p`'s bit and `X` group `2p + 1`'s (lever `z`'s pairing on exclusive controls). The
/// leftover `X Y` (a function of the one-hot) is cancelled once at the end. `2^s - 1 + w 2^(s-1)`
/// Toffolis per `i`.
fn read_groups(
    b: &mut Builder,
    hot: &ItemHot,
    i_reg: &[Qubit],
    n_i: u64,
    out: &[Qubit],
    data: &dyn Fn(u64, u64) -> Word,
) {
    let w = out.len();
    let pairs = hot.groups() / 2;
    let mut xy = vec![vec![false; hot.e1]; w];
    iterate(
        b,
        None,
        i_reg,
        n_i,
        &mut |b: &mut Builder, i: u64, f: Qubit| {
            let u = gate_split(b, f, &hot.glo);
            for (k, &o) in out.iter().enumerate() {
                for p in 0..pairs {
                    let (ga, gb) = (2 * p, 2 * p + 1);
                    let y = support(hot, ga, i, k, data);
                    let x = support(hot, gb, i, k, data);
                    if x.is_empty() && y.is_empty() {
                        continue;
                    }
                    x.iter().for_each(|&c| b.cx(hot.q[c], u[ga]));
                    y.iter().for_each(|&c| b.cx(hot.q[c], u[gb]));
                    if !super::onehot::fault_is(61) || p == 0 {
                        b.ccx(u[ga], u[gb], o);
                    }
                    x.iter().for_each(|&c| b.cx(hot.q[c], u[ga]));
                    y.iter().for_each(|&c| b.cx(hot.q[c], u[gb]));
                    for &c in &x {
                        if y.contains(&c) {
                            xy[k][c] ^= true;
                        }
                    }
                }
            }
            gate_merge(b, &u, &hot.glo);
        },
    );
    for (k, &o) in out.iter().enumerate() {
        for (c, &on) in xy[k].iter().enumerate() {
            if on {
                b.cx(hot.q[c], o);
            }
        }
    }
}

/// Lever `Y`'s [`phase`]: the same products with a `CZ` (conditioned on the outcome) in place of
/// the Toffoli target: `2^s - 1` Toffolis per `i`.
fn phase_groups(b: &mut Builder, hot: &ItemHot, i_reg: &[Qubit], n_i: u64, parts: &[Part<'_, '_>]) {
    let pairs = hot.groups() / 2;
    let mut xy: Vec<Vec<Vec<bool>>> = parts
        .iter()
        .map(|(m, _)| vec![vec![false; hot.e1]; m.len()])
        .collect();
    iterate(
        b,
        None,
        i_reg,
        n_i,
        &mut |b: &mut Builder, i: u64, f: Qubit| {
            let u = gate_split(b, f, &hot.glo);
            for (pi, (m, data)) in parts.iter().enumerate() {
                for (k, &mk) in m.iter().enumerate() {
                    for p in 0..pairs {
                        let (ga, gb) = (2 * p, 2 * p + 1);
                        let y = support(hot, ga, i, k, *data);
                        let x = support(hot, gb, i, k, *data);
                        if x.is_empty() && y.is_empty() {
                            continue;
                        }
                        b.push_condition(mk);
                        x.iter().for_each(|&c| b.cx(hot.q[c], u[ga]));
                        y.iter().for_each(|&c| b.cx(hot.q[c], u[gb]));
                        if !super::onehot::fault_is(62) || p == 0 {
                            b.cz(u[ga], u[gb]);
                        }
                        x.iter().for_each(|&c| b.cx(hot.q[c], u[ga]));
                        y.iter().for_each(|&c| b.cx(hot.q[c], u[gb]));
                        b.pop_condition();
                        for &c in &x {
                            if y.contains(&c) {
                                xy[pi][k][c] ^= true;
                            }
                        }
                    }
                }
            }
            gate_merge(b, &u, &hot.glo);
        },
    );
    for (pi, (m, _)) in parts.iter().enumerate() {
        for (k, &mk) in m.iter().enumerate() {
            let slots: Vec<usize> = (0..hot.e1).filter(|&c| xy[pi][k][c]).collect();
            if slots.is_empty() {
                continue;
            }
            b.push_condition(mk);
            for c in slots {
                b.z(hot.q[c]);
            }
            b.pop_condition();
        }
    }
}

/// Lever `Y`'s [`fan`]: `value(item)` over the slot one-hot and the group bits `x = glo`, by
/// the Moebius expansion over the group: `value = sum_S x_S M_S(h)` with `M_S` the XOR of the
/// group values over the subsets of `S`. `S = {}` is a CNOT fan-out; every other monomial one
/// Toffoli `(x_S, parity, target)` per bit with a nonzero `M_S` (`x_S` an AND of the group bits,
/// made once per call when `|S| >= 2`). Nothing on a lane whose one-hot is empty.
fn fan_groups(b: &mut Builder, hot: &ItemHot, targets: &[Qubit], value: &dyn Fn(u64) -> u64) {
    let s = hot.shift;
    let ng = 1usize << s;
    let at = |g: usize, c: usize| hot.item(g, c).map_or(0, value);
    // Moebius coefficients per slot: m[S][c].
    let m: Vec<Vec<u64>> = (0..ng)
        .map(|set| {
            (0..hot.e1)
                .map(|c| {
                    (0..ng)
                        .filter(|g| g & !set == 0)
                        .fold(0, |acc, g| acc ^ at(g, c))
                })
                .collect()
        })
        .collect();
    for (c, &h) in hot.q.iter().enumerate() {
        for (k, &t) in targets.iter().enumerate() {
            if m[0][c] >> k & 1 == 1 {
                b.cx(h, t);
            }
        }
    }
    // The monomials' control qubits (ANDs for |S| >= 2, made by a chain, erased at the end).
    let mut ctl: Vec<Option<Qubit>> = vec![None; ng];
    let mut made: Vec<(Qubit, Qubit, Qubit)> = Vec::new();
    for set in 1..ng {
        if set.count_ones() == 1 {
            ctl[set] = Some(hot.glo[set.trailing_zeros() as usize]);
            continue;
        }
        let top = 63 - (set as u64).leading_zeros() as usize;
        let rest = set & !(1 << top);
        let a = ctl[rest].expect("subsets come first");
        let q = b.alloc();
        b.ccx(a, hot.glo[top], q);
        made.push((a, hot.glo[top], q));
        ctl[set] = Some(q);
    }
    for set in 1..ng {
        let x = ctl[set].expect("every monomial has a control");
        for (k, &t) in targets.iter().enumerate() {
            let slots: Vec<Qubit> = (0..hot.e1)
                .filter(|&c| m[set][c] >> k & 1 == 1)
                .map(|c| hot.q[c])
                .collect();
            match slots.as_slice() {
                [] => {}
                [h] => b.ccx(x, *h, t),
                _ => {
                    let sc = b.alloc();
                    slots.iter().for_each(|&h| b.cx(h, sc));
                    b.ccx(x, sc, t);
                    slots.iter().for_each(|&h| b.cx(h, sc));
                    b.free(sc);
                }
            }
        }
    }
    for (a, x, q) in made.into_iter().rev() {
        erase_and(b, a, x, q, false);
    }
}

#[cfg(test)]
mod tests {
    //! Exhaustive over every item (in and out of range) and inner index of small random tables:
    //! the read gives the word, the phase cancels a measured copy of it, the one-hot is erased
    //! clean, and each mutant is caught.
    use super::*;
    use crate::walk::sa_low::onehot::FAULT;
    use crate::walk::shared::testsim::Sim;

    thread_local! {
        /// Lever `I` in [`lane`] (aligned only).
        static INPLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// Lever `Y` in [`lane`] (with `INPLACE`): four groups on the item's two low bits.
        static GROUPS4: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

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

    /// One lane: item `v`, inner index `i`; writes, reads, checks, measures the output, applies
    /// the phase, erases the one-hot; returns (the word read, clean?).
    #[allow(clippy::too_many_arguments)]
    fn lane(
        n: usize,
        start: u64,
        n_i: u64,
        w: usize,
        tab: &[Vec<u64>],
        v: u64,
        i: u64,
        seed: u64,
        fault: u8,
        aligned: bool,
    ) -> (u64, bool) {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let item = b.alloc_n(5);
        let ireg = b.alloc_n(3);
        let ob = b.alloc();
        let mut hot = if GROUPS4.with(std::cell::Cell::get) {
            ItemHot::alloc_aligned_groups(&mut b, start, n, &item[..2])
        } else if aligned {
            ItemHot::alloc_aligned(&mut b, start, n, item[0])
        } else {
            ItemHot::alloc(&mut b, start, n)
        };
        if INPLACE.with(std::cell::Cell::get) {
            hot.inplace = Some(ob);
        }
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(w);
            if it >= start && it < start + n as u64 {
                wd[0] = tab[(it - start) as usize][ii as usize];
            }
            wd
        };
        FAULT.with(|c| c.set(fault));
        write(&mut b, &hot, &item);
        let out = read(&mut b, &hot, &ireg, n_i, w, &data);
        let mid = b.ops().len();
        let m: Vec<Bit> = out.iter().map(|&q| b.hmr(q)).collect();
        phase(&mut b, &hot, &ireg, n_i, &[(&m, &data)]);
        erase(&mut b, &hot, &item, 32, Split { h: 2, hot: None });
        FAULT.with(|c| c.set(0));
        let mut sim = Sim::new(&b, seed);
        for (j, &q) in item.iter().enumerate() {
            sim.set(q, v >> j & 1 == 1);
        }
        for (j, &q) in ireg.iter().enumerate() {
            sim.set(q, i >> j & 1 == 1);
        }
        // `is_ob` is 1 exactly on the lanes whose item is out of the register's range.
        let out_of_range = !(v >= start && v < start + n as u64);
        sim.set(ob, out_of_range);
        sim.run(&b.ops()[..mid]);
        let got = sim.read(&out);
        sim.run(&b.ops()[mid..]);
        for &q in item.iter().chain(&ireg) {
            sim.set(q, false);
        }
        sim.set(ob, false);
        let clean = std::panic::catch_unwind(|| sim.assert_clean()).is_ok();
        (got, clean)
    }

    #[test]
    fn item_hot_read_and_phase_are_exact() {
        for (n, start) in [(5usize, 3u64), (6, 0), (9, 7), (13, 2)] {
            for n_i in [3u64, 8] {
                let w = 6;
                let tab = table(n as u64 * 11 + n_i, n, n_i as usize, w);
                for v in 0..24u64 {
                    for i in 0..n_i {
                        let want = if v >= start && v < start + n as u64 {
                            tab[(v - start) as usize][i as usize]
                        } else {
                            0
                        };
                        for seed in 1..4 {
                            for aligned in [false, true] {
                                let (got, clean) =
                                    lane(n, start, n_i, w, &tab, v, i, seed, 0, aligned);
                                assert_eq!(
                                    got, want,
                                    "n {n} start {start} item {v} i {i} {aligned}"
                                );
                                assert!(clean, "n {n} item {v} i {i} seed {seed} {aligned}: dirty");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn item_hot_mutants_are_caught() {
        let (n, start, n_i, w) = (9usize, 7u64, 8u64, 6usize);
        let tab = table(5, n, n_i as usize, w);
        for fault in [30u8, 31, 32, 33] {
            let caught = (0..24u64).any(|v| {
                (0..n_i).any(|i| {
                    let want = if v >= start && v < start + n as u64 {
                        tab[(v - start) as usize][i as usize]
                    } else {
                        0
                    };
                    (1..4).any(|seed| {
                        let (got, clean) = lane(n, start, n_i, w, &tab, v, i, seed, fault, false);
                        got != want || !clean
                    })
                })
            });
            assert!(caught, "item-hot fault {fault} not caught");
        }
    }

    /// Lever `V`: the aligned layout's own mutants are caught (35: a boundary slot written by the
    /// plain flag, so an out-of-range neighbour's one-hot is not empty), and so are the read and
    /// phase faults on it; the aligned write costs about half the contiguous one.
    #[test]
    fn aligned_item_hot_mutants_are_caught_and_it_is_cheaper() {
        for (n, start) in [(9usize, 7u64), (6, 3), (10, 2)] {
            let (n_i, w) = (8u64, 6usize);
            let tab = table(5 + n as u64, n, n_i as usize, w);
            let faults: &[u8] = if start % 2 == 1 || (start + n as u64) % 2 == 1 {
                &[35, 31, 32, 33]
            } else {
                &[31, 32, 33]
            };
            for &fault in faults {
                let caught = (0..24u64).any(|v| {
                    (0..n_i).any(|i| {
                        let want = if v >= start && v < start + n as u64 {
                            tab[(v - start) as usize][i as usize]
                        } else {
                            0
                        };
                        (1..4).any(|seed| {
                            let (got, clean) =
                                lane(n, start, n_i, w, &tab, v, i, seed, fault, true);
                            got != want || !clean
                        })
                    })
                });
                assert!(
                    caught,
                    "n {n} start {start}: aligned fault {fault} not caught"
                );
            }
        }
        let tof = |aligned: bool| {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let item = b.alloc_n(9);
            let hot = if aligned {
                ItemHot::alloc_aligned(&mut b, 54, 270, item[0])
            } else {
                ItemHot::alloc(&mut b, 54, 270)
            };
            let start = b.ops().len();
            write(&mut b, &hot, &item);
            b.ops()[start..]
                .iter()
                .filter(|o| matches!(o.kind, crate::circuit::OperationType::CCX))
                .count()
        };
        let (plain, al) = (tof(false), tof(true));
        assert!(
            al * 2 <= plain + 12,
            "aligned write {al} vs contiguous {plain}"
        );
        println!(
            "item one-hot write, 270 items from 54: contiguous {plain}, aligned {al} Toffolis"
        );
    }

    /// Lever `I`: with the in-place expansion and the measured collapse, every item in and out of
    /// range and every inner index reads its word and ends clean; the collapse has no Toffoli and
    /// the expansion one per split; mutants 36 (a collapse `CZ` skipped) and 37 (the wrong index
    /// bit in it) are caught.
    #[test]
    fn inplace_item_hot_is_exact_and_its_erasure_is_clifford() {
        INPLACE.with(|c| c.set(true));
        for (n, start) in [(5usize, 3u64), (6, 0), (9, 7), (13, 2), (10, 6)] {
            for n_i in [3u64, 8] {
                let w = 6;
                let tab = table(n as u64 * 11 + n_i, n, n_i as usize, w);
                for v in 0..24u64 {
                    for i in 0..n_i {
                        let want = if v >= start && v < start + n as u64 {
                            tab[(v - start) as usize][i as usize]
                        } else {
                            0
                        };
                        for seed in 1..4 {
                            let (got, clean) = lane(n, start, n_i, w, &tab, v, i, seed, 0, true);
                            assert_eq!(got, want, "inplace n {n} start {start} item {v} i {i}");
                            assert!(clean, "inplace n {n} item {v} i {i} seed {seed}: dirty");
                        }
                    }
                }
            }
            for fault in [36u8, 37] {
                let tab = table(3 + n as u64, n, 8, 6);
                let caught = (0..24u64).any(|v| {
                    (0..8).any(|i| {
                        (1..5).any(|seed| {
                            let want = if v >= start && v < start + n as u64 {
                                tab[(v - start) as usize][i as usize]
                            } else {
                                0
                            };
                            let (got, clean) = lane(n, start, 8, 6, &tab, v, i, seed, fault, true);
                            got != want || !clean
                        })
                    })
                });
                assert!(
                    caught || n < 4,
                    "n {n} start {start}: inplace fault {fault} not caught"
                );
            }
        }
        INPLACE.with(|c| c.set(false));
        let count = |inplace: bool, collapse_only: bool| {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let item = b.alloc_n(9);
            let ob = b.alloc();
            let mut hot = ItemHot::alloc_aligned(&mut b, 54, 270, item[0]);
            if inplace {
                hot.inplace = Some(ob);
            }
            write(&mut b, &hot, &item);
            let mid = b.ops().len();
            erase(&mut b, &hot, &item, 324, Split { h: 3, hot: None });
            let from = if collapse_only { mid } else { 0 };
            b.ops()[from..]
                .iter()
                .filter(|o| {
                    matches!(
                        o.kind,
                        crate::circuit::OperationType::CCX | crate::circuit::OperationType::CCZ
                    )
                })
                .count()
        };
        assert_eq!(count(true, true), 0, "the measured collapse is Clifford");
        println!(
            "item one-hot 270 items from 54: write+erase aligned {}, in place {} (erase {} vs {})",
            count(false, false),
            count(true, false),
            count(false, true),
            count(true, true)
        );
    }

    /// Lever `Y`: the four-group aligned register, written in place and collapsed,
    /// reads every word and ends clean for every item in and out of range and every inner index;
    /// its read costs `3 + 2 w` Toffolis per `i` and its phase `3`; mutants 61 (a pair's Toffoli
    /// skipped in the read) and 62 (in the phase), 36 and 37 (the collapse) are caught.
    #[test]
    fn groups4_item_hot_is_exact() {
        INPLACE.with(|c| c.set(true));
        GROUPS4.with(|c| c.set(true));
        let cases = [
            (5usize, 3u64),
            (6, 0),
            (9, 7),
            (13, 2),
            (10, 6),
            (17, 5),
            (22, 9),
        ];
        for (n, start) in cases {
            for n_i in [3u64, 8] {
                let w = 6;
                let tab = table(n as u64 * 13 + n_i, n, n_i as usize, w);
                for v in 0..32u64 {
                    for i in 0..n_i {
                        let want = if v >= start && v < start + n as u64 {
                            tab[(v - start) as usize][i as usize]
                        } else {
                            0
                        };
                        for seed in 1..4 {
                            let (got, clean) = lane(n, start, n_i, w, &tab, v, i, seed, 0, true);
                            assert_eq!(got, want, "Y n {n} start {start} item {v} i {i}");
                            assert!(clean, "Y n {n} item {v} i {i} seed {seed}: dirty");
                        }
                    }
                }
            }
            for fault in [61u8, 62, 36, 37] {
                let tab = table(7 + n as u64, n, 8, 6);
                let caught = (0..32u64).any(|v| {
                    (0..8).any(|i| {
                        (1..5).any(|seed| {
                            let want = if v >= start && v < start + n as u64 {
                                tab[(v - start) as usize][i as usize]
                            } else {
                                0
                            };
                            let (got, clean) = lane(n, start, 8, 6, &tab, v, i, seed, fault, true);
                            got != want || !clean
                        })
                    })
                });
                assert!(
                    caught || n < 9,
                    "Y n {n} start {start}: fault {fault} not caught"
                );
            }
        }
        GROUPS4.with(|c| c.set(false));
        INPLACE.with(|c| c.set(false));
        // Costs, per inner index: read 3 + 2 w (+ the iteration), phase 3.
        let count = |phase_only: bool| {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let item = b.alloc_n(9);
            let ireg = b.alloc_n(3);
            let mut hot = ItemHot::alloc_aligned_groups(&mut b, 54, 270, &item[..2]);
            hot.inplace = Some(b.alloc());
            assert_eq!(hot.e1, 68);
            let tab = table(9, 270, 8, 6);
            let data = |it: u64, ii: u64| -> Word {
                let mut wd = word(6);
                if (54..324).contains(&it) {
                    wd[0] = tab[(it - 54) as usize][ii as usize];
                }
                wd
            };
            let from = b.ops().len();
            if phase_only {
                let m: Vec<Bit> = (0..6)
                    .map(|_| crate::walk::sa_low::narrow::zero_bit(&mut b))
                    .collect();
                phase(&mut b, &hot, &ireg, 8, &[(&m, &data)]);
            } else {
                read(&mut b, &hot, &ireg, 8, 6, &data);
            }
            b.ops()[from..]
                .iter()
                .filter(|o| matches!(o.kind, crate::circuit::OperationType::CCX))
                .count()
        };
        let (rd, ph) = (count(false), count(true));
        println!("Y read {rd}, phase {ph} Toffolis (8 inner indices, w 6)");
        assert!(rd <= 8 * (3 + 2 * 6) + 8, "read {rd}");
        assert!(ph <= 8 * 3 + 8, "phase {ph}");
    }

    /// Lever `Y`'s `fan` (the Moebius expansion over the group bits) gives `value(item)` for every
    /// item in range and nothing out of it, and a second fan clears it.
    #[test]
    fn groups4_fan_is_exact() {
        for (n, start) in [(9usize, 7u64), (13, 2), (22, 9), (17, 5)] {
            for v in 0..32u64 {
                let mut b = Builder::new(1);
                b.declare_uniform(1);
                let item = b.alloc_n(5);
                let ob = b.alloc();
                let tg = b.alloc_n(4);
                let mut hot = ItemHot::alloc_aligned_groups(&mut b, start, n, &item[..2]);
                hot.inplace = Some(ob);
                let value = |it: u64| (it * 7 + 3) % 16;
                write(&mut b, &hot, &item);
                fan(&mut b, &hot, &tg, &value);
                let mid = b.ops().len();
                fan(&mut b, &hot, &tg, &value);
                erase(&mut b, &hot, &item, 32, Split { h: 2, hot: None });
                let mut sim = Sim::new(&b, 1);
                for (j, &q) in item.iter().enumerate() {
                    sim.set(q, v >> j & 1 == 1);
                }
                let inr = v >= start && v < start + n as u64;
                sim.set(ob, !inr);
                sim.run(&b.ops()[..mid]);
                let want = if inr { value(v) } else { 0 };
                assert_eq!(sim.read(&tg), want, "n {n} start {start} item {v}");
                sim.run(&b.ops()[mid..]);
                for &q in &item {
                    sim.set(q, false);
                }
                sim.set(ob, false);
                sim.assert_clean();
            }
        }
    }
}

#[cfg(test)]
mod rooted_tests {
    //! Lever `t`: the rooted read with an in-pass phase, and the rooted erasure,
    //! exhaustively over every item (in and out of range), inner index, root value and three
    //! outcome seeds of small random tables: the read gives `root . word`, the lane ends clean
    //! with phase exactly `(-1)^(root . p(item, i))`, and the mutants 90, 91 and 94 are caught.
    use super::*;
    use crate::walk::sa_low::onehot::FAULT;
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

    /// One lane; returns (the word read, clean with the expected phase removed).
    #[allow(clippy::too_many_arguments)]
    fn lane(
        n: usize,
        start: u64,
        n_i: u64,
        w: usize,
        tab: &[Vec<u64>],
        v: u64,
        i: u64,
        root_on: bool,
        seed: u64,
        fault: u8,
        aligned: bool,
    ) -> (u64, bool) {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let item = b.alloc_n(5);
        let ireg = b.alloc_n(3);
        let root = b.alloc();
        let pq = b.alloc();
        let hot = if aligned {
            ItemHot::alloc_aligned(&mut b, start, n, item[0])
        } else {
            ItemHot::alloc(&mut b, start, n)
        };
        let in_range = |it: u64| it >= start && it < start + n as u64;
        // The word is the table's low `w - 1` bits; the phase is its top bit.
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(w - 1);
            if in_range(it) {
                wd[0] = tab[(it - start) as usize][ii as usize] & ((1 << (w - 1)) - 1);
            }
            wd
        };
        let ph = |it: u64, ii: u64| {
            in_range(it) && tab[(it - start) as usize][ii as usize] >> (w - 1) & 1 == 1
        };
        FAULT.with(|c| c.set(fault));
        write(&mut b, &hot, &item);
        let out = b.alloc_n(w - 1);
        let r = (fault != 94).then_some(root);
        read_into_ext(&mut b, &hot, &ireg, n_i, &out, &data, r, Some(&ph));
        let mid = b.ops().len();
        let m: Vec<Bit> = out.iter().map(|&q| b.hmr(q)).collect();
        phase_rooted(&mut b, &hot, &ireg, n_i, &[(&m, &data)], r);
        erase(&mut b, &hot, &item, 32, Split { h: 2, hot: None });
        b.z(pq);
        FAULT.with(|c| c.set(0));
        let mut sim = Sim::new(&b, seed);
        for (j, &q) in item.iter().enumerate() {
            sim.set(q, v >> j & 1 == 1);
        }
        for (j, &q) in ireg.iter().enumerate() {
            sim.set(q, i >> j & 1 == 1);
        }
        sim.set(root, root_on);
        sim.set(pq, root_on && ph(v, i));
        sim.run(&b.ops()[..mid]);
        let got = sim.read(&out);
        sim.run(&b.ops()[mid..]);
        for &q in item.iter().chain(&ireg).chain([&root, &pq]) {
            sim.set(q, false);
        }
        let clean = std::panic::catch_unwind(|| sim.assert_clean()).is_ok();
        (got, clean)
    }

    #[test]
    fn rooted_read_phase_and_erasure_are_exact() {
        for (n, start) in [(5usize, 3u64), (6, 0), (9, 7), (13, 2)] {
            for n_i in [3u64, 8] {
                let w = 6;
                let tab = table(n as u64 * 13 + n_i, n, n_i as usize, w);
                for v in 0..24u64 {
                    for i in 0..n_i {
                        for root_on in [false, true] {
                            let want = if root_on && v >= start && v < start + n as u64 {
                                tab[(v - start) as usize][i as usize] & 31
                            } else {
                                0
                            };
                            for seed in 1..4 {
                                for aligned in [false, true] {
                                    let (got, clean) = lane(
                                        n, start, n_i, w, &tab, v, i, root_on, seed, 0, aligned,
                                    );
                                    assert_eq!(got, want, "n {n} item {v} i {i} root {root_on}");
                                    assert!(clean, "n {n} item {v} i {i} root {root_on}: dirty");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rooted_mutants_are_caught() {
        let (n, start, n_i, w) = (13usize, 2u64, 8u64, 6usize);
        let tab = table(7, n, n_i as usize, w);
        for fault in [90u8, 91, 94] {
            let caught = (0..24u64).any(|v| {
                (0..n_i).any(|i| {
                    [false, true].iter().any(|&root_on| {
                        (1..3).any(|seed| {
                            let want = if root_on && v >= start && v < start + n as u64 {
                                tab[(v - start) as usize][i as usize] & 31
                            } else {
                                0
                            };
                            let (got, clean) =
                                lane(n, start, n_i, w, &tab, v, i, root_on, seed, fault, false);
                            got != want || !clean
                        })
                    })
                })
            });
            assert!(caught, "rooted fault {fault} not caught");
        }
    }
}

#[cfg(test)]
mod paired_phase_tests {
    //! Lever `.e`: the paired phase pass, exhaustively over every item (in and out of
    //! range), inner index (odd and even counts), root value (and no root), both layouts and
    //! several outcome seeds of small random tables: after the read, the measured outcomes'
    //! phase is cancelled exactly (the lane ends clean with the in-pass phase removed), and the
    //! mutants 130 (the pair's `CCZ` skipped), 131 (the classical left-over skipped) and 132 (the
    //! final left-over phase skipped) are caught.
    use super::*;
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

    /// One lane; returns (the word read, clean with the expected in-pass phase removed).
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
        aligned: bool,
    ) -> (u64, bool) {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let item = b.alloc_n(5);
        let ireg = b.alloc_n(4);
        let root = b.alloc();
        let pq = b.alloc();
        let hot = if aligned {
            ItemHot::alloc_aligned(&mut b, start, n, item[0])
        } else {
            ItemHot::alloc(&mut b, start, n)
        };
        let in_range = |it: u64| it >= start && it < start + n as u64;
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(w - 1);
            if in_range(it) {
                wd[0] = tab[(it - start) as usize][ii as usize] & ((1 << (w - 1)) - 1);
            }
            wd
        };
        let ph = |it: u64, ii: u64| {
            in_range(it) && tab[(it - start) as usize][ii as usize] >> (w - 1) & 1 == 1
        };
        write(&mut b, &hot, &item);
        let out = b.alloc_n(w - 1);
        let r = root_on.map(|_| root);
        read_into_ext(&mut b, &hot, &ireg, n_i, &out, &data, r, Some(&ph));
        let mid = b.ops().len();
        let m: Vec<Bit> = out.iter().map(|&q| b.hmr(q)).collect();
        FAULT_E.with(|c| c.set(fault));
        phase_rooted_paired(&mut b, &hot, &ireg, n_i, &[(&m, &data)], r);
        FAULT_E.with(|c| c.set(0));
        erase(&mut b, &hot, &item, 32, Split { h: 2, hot: None });
        b.z(pq);
        let mut sim = Sim::new(&b, seed);
        for (j, &q) in item.iter().enumerate() {
            sim.set(q, v >> j & 1 == 1);
        }
        for (j, &q) in ireg.iter().enumerate() {
            sim.set(q, i >> j & 1 == 1);
        }
        let on = root_on.unwrap_or(true);
        sim.set(root, root_on == Some(true));
        sim.set(pq, on && ph(v, i));
        sim.run(&b.ops()[..mid]);
        let got = sim.read(&out);
        sim.run(&b.ops()[mid..]);
        for &q in item.iter().chain(&ireg).chain([&root, &pq]) {
            sim.set(q, false);
        }
        let clean = std::panic::catch_unwind(|| sim.assert_clean()).is_ok();
        (got, clean)
    }

    #[test]
    fn paired_phase_pass_is_exact() {
        for (n, start) in [(5usize, 3u64), (6, 0), (9, 7), (13, 2)] {
            for n_i in [3u64, 8, 11] {
                let w = 6;
                let tab = table(n as u64 * 17 + n_i, n, n_i as usize, w);
                for v in 0..24u64 {
                    for i in 0..n_i {
                        for root_on in [None, Some(false), Some(true)] {
                            let on = root_on.unwrap_or(true);
                            let want = if on && v >= start && v < start + n as u64 {
                                tab[(v - start) as usize][i as usize] & 31
                            } else {
                                0
                            };
                            for seed in 1..5 {
                                for aligned in [false, true] {
                                    let (got, clean) = lane(
                                        n, start, n_i, w, &tab, v, i, root_on, seed, 0, aligned,
                                    );
                                    assert_eq!(got, want, "n {n} item {v} i {i} root {root_on:?}");
                                    assert!(
                                        clean,
                                        "n {n} n_i {n_i} item {v} i {i} root {root_on:?} seed {seed}: dirty"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn paired_phase_mutants_are_caught() {
        let (n, start, n_i, w) = (13usize, 2u64, 8u64, 6usize);
        let tab = table(9, n, n_i as usize, w);
        for fault in [130u8, 131, 132] {
            let caught = (0..24u64).any(|v| {
                (0..n_i).any(|i| {
                    (1..6).any(|seed| {
                        let (_, clean) =
                            lane(n, start, n_i, w, &tab, v, i, Some(true), seed, fault, true);
                        !clean
                    })
                })
            });
            assert!(caught, "paired phase fault {fault} not caught");
        }
    }

    /// The pass costs one Toffoli per sibling pair (plus the iteration) where the unpaired pass
    /// costs one per leaf.
    #[test]
    fn paired_phase_pass_is_cheaper() {
        let (n, start, n_i, w) = (13usize, 2u64, 16u64, 6usize);
        let tab = table(3, n, n_i as usize, w);
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(w);
            if it >= start && it < start + n as u64 {
                wd[0] = tab[(it - start) as usize][ii as usize];
            }
            wd
        };
        let count = |paired: bool| {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let item = b.alloc_n(5);
            let ireg = b.alloc_n(4);
            let hot = ItemHot::alloc_aligned(&mut b, start, n, item[0]);
            let m: Vec<Bit> = (0..w)
                .map(|_| super::super::narrow::zero_bit(&mut b))
                .collect();
            let n0 = b.ops().len();
            if paired {
                phase_rooted_paired(&mut b, &hot, &ireg, n_i, &[(&m, &data)], None);
            } else {
                phase_rooted(&mut b, &hot, &ireg, n_i, &[(&m, &data)], None);
            }
            b.ops()[n0..]
                .iter()
                .filter(|o| {
                    matches!(
                        o.kind,
                        crate::circuit::OperationType::CCX | crate::circuit::OperationType::CCZ
                    )
                })
                .count()
        };
        let (c0, c1) = (count(false), count(true));
        assert!(c1 + (n_i as usize) / 2 <= c0 + 1, "paired {c1} vs {c0}");
    }
}

#[cfg(test)]
mod hot_keep_tests {
    //! Lever `~`: the outcome-gated keep re-read from the item one-hot, exhaustively
    //! over every item (in and out of range), inner index, draw and root value of small random
    //! tables, on the three one-hot layouts (contiguous, aligned, four-group in place): the keep
    //! is read, compared, measured; `lt` is measured; the gated block and one erasure pass over
    //! both measured parts must leave every lane clean and phase-free. Mutants 201-206 and the
    //! comparator faults are caught.
    use super::*;
    use crate::walk::common::arith::less_than;
    use crate::walk::sa_low::narrow::{lt_phase_inplace, Fault, Pool};
    use crate::walk::sa_low::onehot::FAULT;
    use crate::walk::shared::testsim::Sim;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Layout {
        Plain,
        Aligned,
        Groups4,
    }

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

    struct Case {
        ops: Vec<crate::circuit::Op>,
        item: Vec<Qubit>,
        ireg: Vec<Qubit>,
        draw: Vec<Qubit>,
        ob: Qubit,
        rq: Qubit,
        /// The ops index after which the first read's register holds the keep.
        mid: usize,
        keep: Vec<Qubit>,
    }

    /// The protocol of `toff::copy` with lever `~`, on one item one-hot.
    #[allow(clippy::too_many_arguments)]
    fn build(
        layout: Layout,
        n: usize,
        start: u64,
        n_i: u64,
        mu: usize,
        tab: &[Vec<u64>],
        rooted: bool,
        fault: Fault,
        fault_no: u8,
    ) -> Case {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let item = b.alloc_n(5);
        let ireg = b.alloc_n(3);
        let draw = b.alloc_n(mu);
        let ob = b.alloc();
        let rq = b.alloc();
        let mut hot = match layout {
            Layout::Plain => ItemHot::alloc(&mut b, start, n),
            Layout::Aligned => ItemHot::alloc_aligned(&mut b, start, n, item[0]),
            Layout::Groups4 => ItemHot::alloc_aligned_groups(&mut b, start, n, &item[..2]),
        };
        if layout == Layout::Groups4 {
            hot.inplace = Some(ob);
        }
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(mu);
            if it >= start && it < start + n as u64 && ii < n_i {
                wd[0] = tab[(it - start) as usize][ii as usize];
            }
            wd
        };
        let root = rooted.then_some(rq);
        FAULT.with(|c| c.set(fault_no));
        write(&mut b, &hot, &item);
        // The PREPARE read: the existing reads (an independent construction) where they exist.
        let keep = b.alloc_n(mu);
        match (layout, rooted) {
            (Layout::Groups4, _) => {
                let mut pool = Pool::default();
                read_pooled(&mut b, &hot, &ireg, n_i, &keep, &data, root, &mut pool);
                pool.close(&mut b);
            }
            _ => read_into_ext(&mut b, &hot, &ireg, n_i, &keep, &data, root, None),
        }
        let mid = b.ops().len();
        let lt = less_than(&mut b, &draw, &keep);
        // `~`: the keep leaves now; `lt` at PREPARE^dagger.
        let mk: Vec<Bit> = keep.iter().map(|&q| b.hmr(q)).collect();
        let m = b.hmr(lt);
        let rr = gated_lt_erase_hot(&mut b, &hot, m, &draw, &ireg, n_i, &data, root, fault);
        let mk_part: Vec<Bit> = if fault_no == 205 { Vec::new() } else { mk };
        let rr_part: Vec<Bit> = if fault_no == 206 { Vec::new() } else { rr };
        let parts: [Part<'_, '_>; 2] = [(&mk_part, &data), (&rr_part, &data)];
        if layout == Layout::Groups4 {
            phase(&mut b, &hot, &ireg, n_i, &parts);
        } else {
            phase_rooted(&mut b, &hot, &ireg, n_i, &parts, root);
        }
        erase(&mut b, &hot, &item, 32, Split { h: 2, hot: None });
        FAULT.with(|c| c.set(0));
        Case {
            ops: b.ops().to_vec(),
            item,
            ireg,
            draw,
            ob,
            rq,
            mid,
            keep,
        }
    }

    /// Runs one lane; `Err` on garbage. Also returns the keep the first read gave.
    #[allow(clippy::too_many_arguments)]
    fn lane(
        c: &Case,
        b: &Builder,
        start: u64,
        n: usize,
        v: u64,
        i: u64,
        d: u64,
        rv: bool,
        seed: u64,
    ) -> Result<(u64, u64), String> {
        let mut sim = Sim::new(b, seed);
        let set = |sim: &mut Sim, qs: &[Qubit], x: u64| {
            for (j, &q) in qs.iter().enumerate() {
                sim.set(q, x >> j & 1 == 1);
            }
        };
        set(&mut sim, &c.item, v);
        set(&mut sim, &c.ireg, i);
        set(&mut sim, &c.draw, d);
        sim.set(c.ob, !(v >= start && v < start + n as u64));
        sim.set(c.rq, rv);
        sim.run(&c.ops[..c.mid]);
        let got = sim.read(&c.keep);
        sim.run(&c.ops[c.mid..]);
        let tof = sim.toffolis;
        set(&mut sim, &c.item, 0);
        set(&mut sim, &c.ireg, 0);
        set(&mut sim, &c.draw, 0);
        sim.set(c.ob, false);
        sim.set(c.rq, false);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sim.assert_clean()))
            .map_err(|_| format!("item {v} i {i} draw {d} root {rv} seed {seed}: garbage"))?;
        Ok((got, tof))
    }

    const SHAPES: [(usize, u64); 3] = [(5, 3), (9, 4), (13, 2)];

    /// Runs every lane of a case; `Err` at the first dirty one.
    #[allow(clippy::too_many_arguments)]
    fn all_lanes(
        layout: Layout,
        n: usize,
        start: u64,
        n_i: u64,
        mu: usize,
        tab: &[Vec<u64>],
        rooted: bool,
        fault: Fault,
        fault_no: u8,
        seeds: u64,
    ) -> Result<(), String> {
        // A throwaway builder with the same uniform layout, for `Sim::new`.
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let c = build(layout, n, start, n_i, mu, tab, rooted, fault, fault_no);
        let items: Vec<u64> = (0..24).collect();
        for &v in &items {
            for i in 0..n_i {
                for d in 0..1u64 << mu {
                    for rv in [false, true] {
                        if !rooted && rv {
                            continue;
                        }
                        for seed in 0..seeds {
                            let (got, _) = lane(&c, &b, start, n, v, i, d, rv, seed * 7919 + 1)?;
                            let live = !rooted || rv;
                            let want = if live && v >= start && v < start + n as u64 {
                                tab[(v - start) as usize][i as usize]
                            } else {
                                0
                            };
                            if fault_no == 0 && fault == Fault::None {
                                assert_eq!(got, want, "{layout:?} item {v} i {i}: first read");
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn gated_hot_keep_erase_is_exact() {
        for layout in [Layout::Plain, Layout::Aligned, Layout::Groups4] {
            for (n, start) in SHAPES {
                if layout == Layout::Groups4 && start % 4 != 0 && n < 9 {
                    continue;
                }
                for n_i in [3u64, 8] {
                    for mu in [1usize, 3] {
                        let tab = table(n as u64 * 13 + n_i + mu as u64, n, n_i as usize, mu);
                        for rooted in [false, true] {
                            if rooted && layout == Layout::Groups4 {
                                continue;
                            }
                            all_lanes(layout, n, start, n_i, mu, &tab, rooted, Fault::None, 0, 3)
                                .unwrap_or_else(|e| {
                                    panic!("{layout:?} n {n} start {start} n_i {n_i} mu {mu} rooted {rooted}: {e}")
                                });
                        }
                    }
                }
            }
        }
    }

    /// Mutants: the comparison's phase dropped (`NoPhase`), only its top carry's `Z` dropped
    /// (`NoTopZ`), the block run on outcome 0 (`FlipOutcome`), the re-read skipped (203), its
    /// paired left-overs not cancelled (204), the early keep outcomes (205) or the re-read's
    /// (206) left out of the erasure pass. Each leaves garbage on some lane of every layout.
    #[test]
    fn gated_hot_keep_mutants_are_caught() {
        let cases: [(Fault, u8); 7] = [
            (Fault::NoPhase, 0),
            (Fault::NoTopZ, 0),
            (Fault::FlipOutcome, 0),
            (Fault::None, 203),
            (Fault::None, 204),
            (Fault::None, 205),
            (Fault::None, 206),
        ];
        for layout in [Layout::Plain, Layout::Aligned, Layout::Groups4] {
            let (n, start, n_i, mu) = (9usize, 4u64, 8u64, 3usize);
            let tab = table(77, n, n_i as usize, mu);
            for (fault, no) in cases {
                let caught =
                    all_lanes(layout, n, start, n_i, mu, &tab, false, fault, no, 3).is_err();
                assert!(caught, "{layout:?}: {fault:?} / {no} not caught");
            }
        }
    }

    /// The in-place comparator alone, exhaustively: `(-1)^[a < b]` exactly, registers restored,
    /// the carry-in clean, `2 (n - 1)` Toffolis.
    #[test]
    fn lt_phase_inplace_is_the_comparison() {
        for n in 1..=4usize {
            let mut b = Builder::new(0);
            b.declare_uniform(2 * n as u32);
            let a: Vec<Qubit> = (0..n as u32).map(|i| b.uniform(i)).collect();
            let bb: Vec<Qubit> = (n as u32..2 * n as u32).map(|i| b.uniform(i)).collect();
            let mut pool = Pool::default();
            let at = b.ops().len();
            lt_phase_inplace(&mut b, &a, &bb, &mut pool, true);
            let tof = b.ops()[at..]
                .iter()
                .filter(|o| o.kind == crate::circuit::OperationType::CCX)
                .count();
            assert_eq!(tof, 2 * (n - 1), "n {n}: Toffolis");
            pool.close(&mut b);
            let lt = less_than(&mut b, &a, &bb);
            b.z(lt);
            crate::walk::common::arith::unless_than(&mut b, &a, &bb, lt);
            let ops = b.ops().to_vec();
            for x in 0..1u64 << (2 * n) {
                let mut s = Sim::new(&b, 1);
                s.set_uniform(x);
                s.run(&ops);
                assert_eq!(s.read(&[&a[..], &bb[..]].concat()), x, "n {n}: restored");
                s.assert_clean();
            }
            // Without the top carry's Z (n >= 2) some value is wrong.
            if n >= 2 {
                let mut b2 = Builder::new(0);
                b2.declare_uniform(2 * n as u32);
                let mut pool = Pool::default();
                lt_phase_inplace(&mut b2, &a, &bb, &mut pool, false);
                pool.close(&mut b2);
                let lt = less_than(&mut b2, &a, &bb);
                b2.z(lt);
                crate::walk::common::arith::unless_than(&mut b2, &a, &bb, lt);
                let ops = b2.ops().to_vec();
                let bad = (0..1u64 << (2 * n)).any(|x| {
                    let mut s = Sim::new(&b2, 1);
                    s.set_uniform(x);
                    s.run(&ops);
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.assert_clean()))
                        .is_err()
                });
                assert!(bad, "n {n}: the top Z matters");
            }
        }
    }

    /// The pooled read (folded pair iteration) costs what the four-group read costs and holds
    /// fewer ANDs at once: `log2(4 n_i) - 1 + 1` against `log2(n_i) - 1 + 3`.
    #[test]
    fn pooled_read_costs_like_read_groups() {
        let (n, start, n_i, mu) = (13usize, 4u64, 8u64, 3usize);
        let tab = table(5, n, n_i as usize, mu);
        let data = |it: u64, ii: u64| -> Word {
            let mut wd = word(mu);
            if it >= start && it < start + n as u64 && ii < n_i {
                wd[0] = tab[(it - start) as usize][ii as usize];
            }
            wd
        };
        let count = |pooled: bool| -> (usize, usize) {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let item = b.alloc_n(5);
            let ireg = b.alloc_n(3);
            let hot = ItemHot::alloc_aligned_groups(&mut b, start, n, &item[..2]);
            let out = b.alloc_n(mu);
            let at = b.ops().len();
            let mut pool = Pool::default();
            if pooled {
                read_pooled(&mut b, &hot, &ireg, n_i, &out, &data, None, &mut pool);
            } else {
                read_groups(&mut b, &hot, &ireg, n_i, &out, &data);
            }
            let ops = &b.ops()[at..];
            let tof = ops
                .iter()
                .filter(|o| o.kind == crate::circuit::OperationType::CCX)
                .count();
            // The pool's size is the most pooled ANDs live at once.
            let size = if pooled {
                let before = b.ops().len();
                pool.close(&mut b);
                b.ops().len() - before
            } else {
                0
            };
            (tof, size)
        };
        let (t_groups, _) = count(false);
        let (t_pooled, live) = count(true);
        assert_eq!(t_pooled, t_groups, "same Toffolis");
        // 4 index bits (the high group bit folded in) -> 3 nested ANDs, plus the pair control
        // (`read_groups` holds 2 + 3).
        assert_eq!(live, 4, "ANDs live at once");
    }
}
