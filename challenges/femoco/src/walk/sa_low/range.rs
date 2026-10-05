//! Unary iteration over a range `start..limit` of index values (Babbush et al. 2018, Sec.
//! III.A, with the tree pruned on both sides): a subtree that lies wholly below `start` or at or
//! above `limit` is never entered, so the cost is about `limit - start` Toffolis plus the tree's
//! two boundary paths. Lanes whose index is outside the range get no indicator.
//!
//! With `start = 0` it emits exactly what `shared::unary::iterate` emits.
//!
//! Also: `sa-toff`'s padded write-all-words inner read (lever `p`, section below).
use super::qroam::{self, Read, Split};
use crate::circuit::{Bit, Builder, Qubit};
use crate::lanemap::df_nested::Table;
use crate::walk::shared::arith::cswap_reg;
use crate::walk::shared::lookup::{measure, set_bits, word, Data, Word};
use crate::walk::shared::unary::{erase_and, Leaf};

/// Iterates over `index` (little-endian) for the values `start..limit`.
///
/// # Panics
/// If `index` is empty and there is no control.
pub fn iterate_range(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    start: u64,
    limit: u64,
    leaf: &mut Leaf<'_>,
) {
    assert!(ctl.is_some() || !index.is_empty(), "nothing to iterate on");
    if start < limit {
        node(b, ctl, index, 0, start, limit, leaf);
    }
}

fn node(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    base: u64,
    start: u64,
    limit: u64,
    leaf: &mut Leaf<'_>,
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
            node(b, Some(bit), low, base, start, limit, leaf);
            b.x(bit);
        }
        if right {
            node(b, Some(bit), low, base + half, start, limit, leaf);
        }
        return;
    };
    let child = b.alloc();
    if left {
        // child = c AND NOT bit.
        b.x(bit);
        b.ccx(c, bit, child);
        b.x(bit);
        node(b, Some(child), low, base, start, limit, leaf);
        if right {
            b.cx(c, child);
            node(b, Some(child), low, base + half, start, limit, leaf);
            erase_and(b, c, bit, child, false);
        } else {
            erase_and(b, c, bit, child, true);
        }
    } else {
        // Only the right half is in range: child = c AND bit.
        b.ccx(c, bit, child);
        node(b, Some(child), low, base + half, start, limit, leaf);
        erase_and(b, c, bit, child, false);
    }
}

/// Toffolis of [`iterate_range`] without a control.
#[must_use]
pub fn range_cost(bits: usize, start: u64, limit: u64) -> u64 {
    fn node(ctl: bool, bits: usize, base: u64, start: u64, limit: u64) -> u64 {
        if bits == 0 {
            return 0;
        }
        let half = 1u64 << (bits - 1);
        let left = base + half > start;
        let right = base + half < limit;
        let own = u64::from(ctl);
        let l = if left {
            node(true, bits - 1, base, start, limit)
        } else {
            0
        };
        let r = if right {
            node(true, bits - 1, base + half, start, limit)
        } else {
            0
        };
        own + l + r
    }
    let limit = limit.min(1u64 << bits.min(63));
    if bits == 0 || start >= limit {
        return 0;
    }
    node(false, bits, 0, start, limit)
}

// ---------------------------------------------------------------------------------------------
// The write-all-words inner lookup with shared padding words (`sa-toff` lever `p`;
// README.md here).
//
// Low et al. 2025 App. B Eq. (B38) reads a square's inner alias words by iterating over the
// squares, writing all `B + 1` words at once and swapping one out: `R C + B' w` Toffolis. Here
// the inner table has `2^k_i >= B + 1` buckets (the lane map's uniform register is a power of
// two), and the padding buckets `B + 1 .. 2^k_i` carry weight (keep 0, all lanes to their alt),
// so all `2^k_i` words are data: the QROAM with `a = k_i` blocks already is Eq. (B38) with
// `B' = 2^k_i` (`FEMOCO_SA_INNER_A = k_i`). What this lever adds is the part of `B' = B + 1` that
// survives the padding: the walk builds each square's table so that the aligned top block of
// `2^s` padding buckets holds words that depend only on the bucket's low `d` bits (their alts
// are chosen so, which needs `2^d` items with at least `2^(s-d) 2^mu` lanes each), and the swap
// network (low index bits first) then never swaps two identical words: `2^s - 2^d` blocks are
// never allocated and `2^s - 2^d` controlled swaps are skipped, `(2^s - 2^d) w` Toffolis and
// qubits per read. Counts are unchanged (the same largest-remainder counts), so the lane map's
// rounding error and its floor-or-ceiling check are unchanged; only the alias layout differs.

/// The padding layout of a `2^a`-block read: the aligned top block `base .. 2^a` (`base = 2^a -
/// 2^s`) holds words that depend only on the low `d` bits of the bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadPlan {
    pub a: usize,
    pub s: usize,
    pub d: usize,
}

impl PadPlan {
    /// The largest aligned all-padding top block for `items` real buckets in `2^a`, with words
    /// shared down to `2^d` distinct ones; `None` when there is no padding to share (`s <= d`).
    #[must_use]
    pub fn new(a: usize, items: usize, d: usize) -> Option<Self> {
        let pads = (1usize << a).checked_sub(items)?;
        if pads == 0 {
            return None;
        }
        let s = (usize::BITS - 1 - pads.leading_zeros()) as usize;
        (s > d).then_some(Self { a, s, d })
    }
    /// First bucket of the shared top block.
    #[must_use]
    pub fn base(&self) -> usize {
        (1 << self.a) - (1 << self.s)
    }
    /// Whether bucket `p` has a block of its own (every bucket below `base`, and the first `2^d`
    /// of the top block).
    #[must_use]
    pub fn live(&self, p: usize) -> bool {
        p < self.base() + (1 << self.d)
    }
    /// The bucket whose word `p`'s word equals by construction.
    #[must_use]
    pub fn canon(&self, p: usize) -> usize {
        if p < self.base() {
            p
        } else {
            self.base() + ((p - self.base()) & ((1 << self.d) - 1))
        }
    }
    /// Blocks allocated.
    #[must_use]
    pub fn blocks(&self) -> usize {
        self.base() + (1 << self.d)
    }
    /// Blocks and controlled swaps saved against the full `2^a`-block read.
    #[must_use]
    pub fn saved(&self) -> usize {
        (1 << self.s) - (1 << self.d)
    }
    /// The swaps of the pruned network, low index bits first: `(t, x, y)` swaps blocks `x` and
    /// `y = x + 2^t` under index bit `t`, for every aligned pair whose two blocks are live.
    #[must_use]
    pub fn swaps(&self) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for t in 0..self.a {
            let step = 1usize << (t + 1);
            let mut x = 0;
            while x < 1 << self.a {
                let y = x + (1 << t);
                if self.live(x) && self.live(y) {
                    out.push((t, x, y));
                }
                x += step;
            }
        }
        out
    }
    /// Where each block's final content came from, for the lane whose low index bits are `low`:
    /// `perm[p]` is the bucket block `p` holds after the network (`perm[0]` holds a bucket whose
    /// word equals bucket `low`'s).
    #[must_use]
    pub fn perm(&self, low: u64) -> Vec<usize> {
        let mut lab: Vec<usize> = (0..1usize << self.a).collect();
        for (t, x, y) in self.swaps() {
            if low >> t & 1 == 1 {
                lab.swap(x, y);
            }
        }
        (0..self.blocks()).map(|p| lab[p]).collect()
    }
}

/// The pad plan every square table of the walk can meet at keep bits `mu`: the smallest `d` for
/// which each table's largest-remainder counts leave `2^d` items (greedily, largest first) with
/// at least `2^(s - d) 2^mu` lanes each. `None` if no `d < s` fits every table.
#[must_use]
pub fn pad_plan(tables: &[Table], items: usize) -> Option<PadPlan> {
    let t0 = tables.first()?;
    let a = t0.k as usize;
    for d in 0..a {
        let plan = PadPlan::new(a, items, d)?;
        if tables.iter().all(|t| pad_owners(t, items, plan).is_some()) {
            return Some(plan);
        }
    }
    None
}

/// The `2^d` owner items of the top block's word classes for table `t` (class `g` = buckets
/// `base + g + m 2^d`), greedily from the largest count; `None` if they do not fit.
fn pad_owners(t: &Table, items: usize, plan: PadPlan) -> Option<Vec<usize>> {
    let cap = 1u64 << t.mu;
    let need = cap << (plan.s - plan.d);
    let mut left = t.counts(items);
    let mut owners = Vec::with_capacity(1 << plan.d);
    for _ in 0..1 << plan.d {
        let (best, &n) = left
            .iter()
            .enumerate()
            .max_by_key(|&(i, &n)| (n, usize::MAX - i))?;
        if n < need {
            return None;
        }
        left[best] -= need;
        owners.push(best);
    }
    Some(owners)
}

/// Rebuilds `t` (same `k`, `mu` and counts over `items` items) so that its top block follows
/// `plan`: the class-`g` padding buckets all send their lanes to owner `g`, and the other
/// buckets are an integer Walker table of the remaining counts.
///
/// # Panics
/// If the plan does not fit this table's counts.
#[must_use]
pub fn pad_table(t: &Table, items: usize, plan: PadPlan) -> Table {
    let cap = 1u64 << t.mu;
    let counts = t.counts(items);
    let owners = pad_owners(t, items, plan).expect("pad plan fits the table");
    let need = cap << (plan.s - plan.d);
    let buckets = 1usize << t.k;
    let base = plan.base();
    let mut left: Vec<u64> = (0..buckets)
        .map(|i| counts.get(i).copied().unwrap_or(0))
        .collect();
    let mut keep = vec![0u32; buckets];
    let mut alt: Vec<u32> = (0..buckets as u32).collect();
    for (g, &o) in owners.iter().enumerate() {
        left[o] -= need;
        for p in (base..buckets).filter(|p| (p - base) & ((1 << plan.d) - 1) == g) {
            alt[p] = o as u32;
        }
    }
    // Walker over the buckets below `base` (own item = bucket; padding ones own nothing).
    let mut small: Vec<usize> = (0..base).filter(|&i| left[i] < cap).collect();
    let mut large: Vec<usize> = (0..buckets).filter(|&i| left[i] > cap).collect();
    // An item past `base` never owns lanes (it is padding), so `large` only holds real items.
    assert!(
        large.iter().all(|&i| i < base),
        "padding bucket with surplus"
    );
    while let Some(s) = small.pop() {
        let l = *large
            .last()
            .expect("Walker: a small bucket needs a large one");
        keep[s] = u32::try_from(left[s]).expect("keep fits");
        alt[s] = l as u32;
        left[l] -= cap - left[s];
        if left[l] <= cap {
            large.pop();
            if left[l] < cap {
                small.push(l);
            }
        }
    }
    let out = Table {
        k: t.k,
        mu: t.mu,
        keep,
        alt,
    };
    assert_eq!(out.counts(items), counts, "pad table keeps the counts");
    out
}

/// A read by [`load_range_pad`]: the output, the junk outcomes and the layout.
pub struct PadRead {
    pub out: Vec<Qubit>,
    junk: Vec<Bit>,
    plan: PadPlan,
    w: usize,
    start: u64,
    limit: u64,
    full: u64,
}

/// `qroam::load_range` with `2^a = 2^plan.a` blocks laid out by `plan`: only the live blocks are
/// allocated and written, and the pruned network (`PadPlan::swaps`) moves the lane's word into
/// block 0. `data` must meet the plan (every top-block word equal to its class's; checked).
///
/// # Panics
/// If `index` has fewer than `plan.a` bits, `start` is not block aligned, or `data` breaks the
/// plan.
#[allow(clippy::too_many_arguments)]
pub fn load_range_pad(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &Data<'_>,
    plan: PadPlan,
) -> PadRead {
    let a = plan.a;
    assert!(index.len() > a, "a padded read needs high index bits");
    let (low, high) = index.split_at(a);
    let lam = 1u64 << a;
    assert!(
        start.is_multiple_of(lam),
        "range start must be block aligned"
    );
    let hi_limit = limit.div_ceil(lam);
    let full = (hi_limit * lam).min(1u64 << index.len().min(63));
    for hv in start / lam..hi_limit {
        for p in plan.base()..lam as usize {
            let (x, c) = (hv * lam + p as u64, hv * lam + plan.canon(p) as u64);
            let (dx, dc) = if (start..limit).contains(&x) {
                (data(x), data(c))
            } else {
                (word(w), word(w))
            };
            assert!(
                set_bits(&dx, w).eq(set_bits(&dc, w)),
                "padded read: bucket {p} of block {hv} breaks the pad plan"
            );
        }
    }
    let nb = plan.blocks();
    let blocks: Vec<Vec<Qubit>> = (0..nb).map(|_| b.alloc_n(w)).collect();
    let flat: Vec<Qubit> = blocks.concat();
    let wide = |hv: u64| -> Word {
        let mut out = word(nb * w);
        for j in 0..nb {
            let x = hv * lam + j as u64;
            if (start..limit).contains(&x) {
                for bit in set_bits(&data(x), w) {
                    let at = j * w + bit;
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
    for (t, x, y) in plan.swaps() {
        cswap_reg(b, low[t], &blocks[x], &blocks[y]);
    }
    let mut it = blocks.into_iter();
    let out = it.next().expect("at least one block");
    let rest: Vec<Qubit> = it.flatten().collect();
    let junk = measure(b, rest);
    PadRead {
        out,
        junk,
        plan,
        w,
        start,
        limit,
        full,
    }
}

/// The junk's phase data of a [`PadRead`] at index `x`: block `p >= 1` held `orig` of the bucket
/// `perm[p]` of `x`'s block.
fn pad_junk(r: &PadRead, orig: &Data<'_>, x: u64, perms: &[Vec<usize>], out: &mut Word) {
    let lam = 1u64 << r.plan.a;
    let (hv, l) = (x / lam, x % lam);
    for (pos, &j) in perms[l as usize].iter().enumerate().skip(1) {
        let y = hv * lam + j as u64;
        if (r.start..r.limit).contains(&y) {
            for bit in set_bits(&orig(y), r.w) {
                let at = (pos - 1) * r.w + bit;
                out[at / 64] |= 1 << (at % 64);
            }
        }
    }
}

/// `qroam::erase_parts` for a [`PadRead`]: one fixup for the junk (`orig`), the measured output
/// (`content`) and `extra` bits (`extra_data`).
#[allow(clippy::too_many_arguments)]
pub fn erase_parts_pad(
    b: &mut Builder,
    index: &[Qubit],
    r: PadRead,
    orig: &Data<'_>,
    out_bits: Vec<Bit>,
    content: &Data<'_>,
    extra: Vec<Bit>,
    extra_data: &Data<'_>,
    s: Split,
) {
    assert_eq!(out_bits.len(), r.w, "one outcome per output slot");
    let lam = 1u64 << r.plan.a;
    let perms: Vec<Vec<usize>> = (0..lam).map(|l| r.plan.perm(l)).collect();
    let junk_w = (r.plan.blocks() - 1) * r.w;
    let (w, ex) = (r.w, extra.len());
    let total = junk_w + w + ex;
    let all = |x: u64| -> Word {
        let mut out = word(total);
        pad_junk(&r, orig, x, &perms, &mut out);
        if (r.start..r.limit).contains(&x) {
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
    let mut bits = r.junk.clone();
    bits.extend(out_bits);
    bits.extend(extra);
    if r.start == 0 {
        qroam::fixup(b, index, r.full, &bits, &all, s);
    } else {
        qroam::fixup_range(b, index, r.start, r.full, &bits, &all, s);
    }
}

/// `qroam::erase_split_by` for a [`PadRead`] (the output still holds `data(x)`).
pub fn erase_split_by_pad(b: &mut Builder, index: &[Qubit], r: PadRead, data: &Data<'_>, s: Split) {
    let out = measure(b, r.out.clone());
    let none = |_: u64| word(0);
    erase_parts_pad(b, index, r, data, out, data, Vec::new(), &none, s);
}

/// The inner read of `sa-toff`: `qroam::load_range` / `qroam::load` as before, or the padded
/// read when `plan` is set.
pub enum InnerRead {
    Qroam(Read),
    Pad(PadRead),
}

impl InnerRead {
    /// The output register (the lane's word).
    #[must_use]
    pub fn out(&self) -> &[Qubit] {
        match self {
            Self::Qroam(r) => &r.out,
            Self::Pad(r) => &r.out,
        }
    }
}

/// `sa-toff`'s inner read: exactly the calls it made before (`plan` `None`), or
/// [`load_range_pad`].
#[allow(clippy::too_many_arguments)]
pub fn inner_load(
    b: &mut Builder,
    index: &[Qubit],
    start: u64,
    limit: u64,
    w: usize,
    data: &Data<'_>,
    a: usize,
    plan: Option<PadPlan>,
) -> InnerRead {
    match plan {
        Some(plan) => InnerRead::Pad(load_range_pad(b, index, start, limit, w, data, plan)),
        None if start > 0 => {
            InnerRead::Qroam(qroam::load_range(b, index, start, limit, w, data, a))
        }
        None => InnerRead::Qroam(qroam::load(b, index, limit, w, data, a)),
    }
}

/// `qroam::erase_parts` or [`erase_parts_pad`], by the read's kind.
#[allow(clippy::too_many_arguments)]
pub fn inner_erase_parts(
    b: &mut Builder,
    index: &[Qubit],
    r: InnerRead,
    orig: &Data<'_>,
    out_bits: Vec<Bit>,
    content: &Data<'_>,
    extra: Vec<Bit>,
    extra_data: &Data<'_>,
    s: Split,
) {
    match r {
        InnerRead::Qroam(r) => {
            qroam::erase_parts(b, index, r, orig, out_bits, content, extra, extra_data, s);
        }
        InnerRead::Pad(r) => {
            erase_parts_pad(b, index, r, orig, out_bits, content, extra, extra_data, s);
        }
    }
}

/// `qroam::erase_split_by` or [`erase_split_by_pad`], by the read's kind.
pub fn inner_erase_split_by(
    b: &mut Builder,
    index: &[Qubit],
    r: InnerRead,
    data: &Data<'_>,
    s: Split,
) {
    match r {
        InnerRead::Qroam(r) => qroam::erase_split_by(b, index, r, data, s),
        InnerRead::Pad(r) => erase_split_by_pad(b, index, r, data, s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every plan up to 2^7 blocks: the pruned network puts a word equal to bucket `low`'s in
    /// block 0 for every `low`, and it has `2^a - 1 - saved` swaps.
    #[test]
    fn pruned_network_selects_every_bucket() {
        for a in 1..=7usize {
            for items in 1..1usize << a {
                for d in 0..a {
                    let Some(plan) = PadPlan::new(a, items, d) else {
                        continue;
                    };
                    assert!(plan.base() >= items, "the top block is all padding");
                    assert_eq!(plan.swaps().len(), (1 << a) - 1 - plan.saved());
                    for low in 0..1u64 << a {
                        let perm = plan.perm(low);
                        assert_eq!(perm.len(), plan.blocks());
                        assert_eq!(
                            plan.canon(perm[0]),
                            plan.canon(low as usize),
                            "{plan:?} {low}"
                        );
                        // The junk is the other live buckets (each once) plus equal copies.
                        let mut seen: Vec<usize> = perm.iter().map(|&j| plan.canon(j)).collect();
                        seen.sort_unstable();
                        let mut want: Vec<usize> = (0..plan.blocks()).collect();
                        want.sort_unstable();
                        assert_eq!(seen, want, "{plan:?} {low}");
                    }
                }
            }
        }
    }

    /// `pad_table` keeps every count, puts the top block's classes on single owners and
    /// otherwise stays an alias table (keeps below `2^mu`, padding below `base` keep 0).
    #[test]
    fn pad_tables_keep_their_counts() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut rnd = |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        let mut fitted = [0usize; 3];
        for trial in 0..400 {
            let (k, mu) = (3 + (trial % 3) as u32, 2 + (trial % 4) as u32);
            let items = (1usize << k) - 2 - rnd(3) as usize;
            let lanes = 1u64 << (k + mu);
            // Random counts over `items` items summing to `lanes`, skewed so a plan usually fits.
            let mut counts = vec![0u64; items];
            let mut left = lanes;
            for c in counts.iter_mut().take(items - 1) {
                let v = rnd(left / 3 + 1);
                *c = v;
                left -= v;
            }
            counts[items - 1] = left;
            let t = Table::from_counts(k, mu, &counts).unwrap();
            for (d, fit) in fitted.iter_mut().enumerate() {
                let Some(plan) = PadPlan::new(k as usize, items, d) else {
                    continue;
                };
                if pad_owners(&t, items, plan).is_none() {
                    continue;
                }
                *fit += 1;
                let p = pad_table(&t, items, plan);
                assert_eq!(p.counts(items), t.counts(items));
                assert!(p.keep.iter().all(|&kp| kp < 1 << mu));
                for q in items..1 << k {
                    assert_eq!(p.keep[q], 0, "padding keeps nothing");
                }
                for q in plan.base()..1 << k {
                    assert_eq!(p.alt[q], p.alt[plan.canon(q)], "{plan:?}");
                }
            }
        }
        // d = 2 needs 8 padding buckets, which these tables never have.
        assert!(fitted[0] > 20 && fitted[1] > 20, "{fitted:?}");
    }
}
