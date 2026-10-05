//! Lever `+`: **donor-padded inner tables and the item-constant
//! offset** for the item one-hot read (`itemhot.rs`, lever `H`).
//!
//! A square item's inner table has `2^k_i` buckets for `B + 1` inner items; the `2^k_i - B - 1`
//! padding buckets keep nothing (`keep = 0`) and alias all their lanes to some item. The lane map
//! fixes only the counts (each the floor or ceiling of its ideal under the estimated class), not
//! which bucket feeds which item, so the walk may choose the arrangement:
//!
//! - **Arrangement** ([`arrange`]): every padding bucket aliases to the item's largest inner item
//!   `D(item)` as long as `D` still has at least `2^mu` lanes to give, from the top padding bucket
//!   down (the rest to the largest remaining item); the `B + 1` own buckets are then a plain
//!   Vose alias over the residual counts (self-aliased buckets `keep = 0`). The counts are the
//!   original table's exactly, so the rounding error and every class check are unchanged.
//! - **Offset** (in `toff.rs`): the padding word `A(item) = word(item, top)` is a function of the
//!   item alone, so the read loads `word(item, i) ^ A(item)` (zero on every padding bucket that
//!   aliases to `D`) over the inner indices below the first all-zero padding row only, and adds
//!   `A(item)` back from the item one-hot under the walk control ([`super::itemhot::fan_rooted`]:
//!   one AND and one paired Toffoli per word bit). Phases (lever `t`'s in-pass sign, the
//!   erasure's measured content) are offset the same way with Clifford corrections.
//!
//! Cost: each skipped padding index saves the iteration's ANDs and one Toffoli per word bit in
//! the read and two ANDs in the erasure pass, against `1 + w_A` Toffolis for the offset.
use crate::lanemap::df_nested::Table;

/// The table realizing `t`'s counts (`items` inner items) with every padding bucket aliased to
/// the largest item while it has `2^mu` lanes left (from the top bucket down), then the
/// largest remaining one; own buckets by Vose's method over the residual counts.
///
/// # Panics
/// If the counts cannot be realized (they always can: they sum to `2^(k + mu)`).
#[must_use]
pub fn arrange(t: &Table, items: usize) -> Table {
    let cap = 1u64 << t.mu;
    let buckets = 1usize << t.k;
    let counts = t.counts(items);
    let mut res: Vec<u64> = counts.clone();
    let mut keep = vec![0u32; buckets];
    let mut alt = vec![0u32; buckets];
    let big = (0..items)
        .max_by_key(|&j| (res[j], usize::MAX - j))
        .unwrap();
    for p in (items..buckets).rev() {
        let d = if res[big] >= cap {
            big
        } else {
            (0..items)
                .max_by_key(|&j| (res[j], usize::MAX - j))
                .unwrap()
        };
        assert!(
            res[d] >= cap,
            "a padding bucket needs a donor with 2^mu lanes"
        );
        res[d] -= cap;
        alt[p] = d as u32;
    }
    // Vose over the own buckets 0..items (residuals sum to items * cap).
    let mut small: Vec<usize> = (0..items).filter(|&j| res[j] < cap).collect();
    let mut large: Vec<usize> = (0..items).filter(|&j| res[j] >= cap).collect();
    small.reverse();
    large.reverse();
    let mut done = vec![false; items];
    while let Some(s) = small.pop() {
        let Some(&l) = large.last() else {
            panic!("residual counts do not fit");
        };
        keep[s] = u32::try_from(res[s]).unwrap();
        alt[s] = l as u32;
        done[s] = true;
        res[l] -= cap - res[s];
        if res[l] < cap {
            large.pop();
            small.push(l);
        }
    }
    for j in 0..items {
        if !done[j] {
            assert_eq!(res[j], cap, "a left-over bucket must be exactly full");
            keep[j] = 0;
            alt[j] = j as u32;
        }
    }
    let out = Table {
        k: t.k,
        mu: t.mu,
        keep,
        alt,
    };
    assert_eq!(
        out.counts(items),
        counts,
        "the arrangement keeps every count"
    );
    out
}

/// The aligned subcubes `[base, base + len)` (`len` a power of two, `base` a multiple of it)
/// that tile `items..buckets`, largest first (lever `.p`).
#[must_use]
pub fn pad_cubes(items: usize, buckets: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = items;
    while at < buckets {
        let mut len = 1usize;
        while at.is_multiple_of(2 * len) && at + 2 * len <= buckets {
            len *= 2;
        }
        out.push((at, len));
        at += len;
    }
    out.sort_by_key(|&(b, l)| (std::cmp::Reverse(l), std::cmp::Reverse(b)));
    out
}

/// Lever `.p` (the paired read's merged padding slots): the table realizing `t`'s
/// counts in which every aligned padding subcube of [`pad_cubes`] aliases to one donor item (the
/// item with the most lanes left, which must hold the whole subcube's `len 2^mu` lanes; a
/// subcube no item can feed is halved), so the padding words are constant on each subcube that
/// fed; then a plain Vose alias over the own buckets' residual counts (as [`arrange`]). The
/// counts are `t`'s exactly. Returns the table and the subcubes as fed (`(base, len)`).
///
/// # Panics
/// If the counts cannot be realized (they always can: they sum to `2^(k + mu)`).
#[must_use]
pub fn arrange_runs(t: &Table, items: usize) -> (Table, Vec<(usize, usize)>) {
    let cap = 1u64 << t.mu;
    let buckets = 1usize << t.k;
    let counts = t.counts(items);
    let mut res: Vec<u64> = counts.clone();
    let mut keep = vec![0u32; buckets];
    let mut alt = vec![0u32; buckets];
    let mut todo = pad_cubes(items, buckets);
    let mut fed = Vec::new();
    while let Some((base, len)) = (!todo.is_empty()).then(|| todo.remove(0)) {
        let d = (0..items)
            .max_by_key(|&j| (res[j], usize::MAX - j))
            .unwrap();
        if res[d] >= len as u64 * cap {
            res[d] -= len as u64 * cap;
            for a in &mut alt[base..base + len] {
                *a = d as u32;
            }
            fed.push((base, len));
        } else {
            assert!(len > 1, "a padding bucket needs a donor with 2^mu lanes");
            let h = len / 2;
            todo.insert(0, (base + h, h));
            todo.insert(0, (base, h));
        }
    }
    let mut small: Vec<usize> = (0..items).filter(|&j| res[j] < cap).collect();
    let mut large: Vec<usize> = (0..items).filter(|&j| res[j] >= cap).collect();
    small.reverse();
    large.reverse();
    let mut done = vec![false; items];
    while let Some(s) = small.pop() {
        let Some(&l) = large.last() else {
            panic!("residual counts do not fit");
        };
        keep[s] = u32::try_from(res[s]).unwrap();
        alt[s] = l as u32;
        done[s] = true;
        res[l] -= cap - res[s];
        if res[l] < cap {
            large.pop();
            small.push(l);
        }
    }
    for j in 0..items {
        if !done[j] {
            assert_eq!(res[j], cap, "a left-over bucket must be exactly full");
            keep[j] = 0;
            alt[j] = j as u32;
        }
    }
    let out = Table {
        k: t.k,
        mu: t.mu,
        keep,
        alt,
    };
    assert_eq!(
        out.counts(items),
        counts,
        "the arrangement keeps every count"
    );
    (out, fed)
}

/// The padding buckets (from the top) that alias to the top padding bucket's item in every
/// table: the inner indices the offset read can skip are `2^k - skippable..2^k` (as far as the
/// alias target is concerned; the caller checks the words themselves).
#[must_use]
pub fn uniform_top(tables: &[Table], items: usize) -> usize {
    let Some(t0) = tables.first() else { return 0 };
    let buckets = 1usize << t0.k;
    (0..buckets - items)
        .take_while(|&d| {
            let p = buckets - 1 - d;
            tables.iter().all(|t| t.alt[p] == t.alt[buckets - 1])
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *seed >> 33
    }

    /// Random count vectors (heavy-tailed, with zeros and exact multiples of `2^mu`): the
    /// arrangement keeps every count, the padding keeps nothing, and as many top padding
    /// buckets as the largest item can feed alias to it.
    #[test]
    fn arrange_keeps_counts_and_pads_to_the_largest() {
        let mut seed = 7u64;
        for k in 2..=6u32 {
            for mu in 1..=9u32 {
                let cap = 1u64 << mu;
                let buckets = 1usize << k;
                for items in (buckets / 2 + 1).max(1)..=buckets {
                    for _ in 0..20 {
                        let total = (buckets as u64) * cap;
                        let mut w: Vec<u64> = (0..items)
                            .map(|_| {
                                let r = lcg(&mut seed) % 100;
                                if r < 15 {
                                    0
                                } else if r < 25 {
                                    cap
                                } else {
                                    1 + lcg(&mut seed) % (1 << (lcg(&mut seed) % 12))
                                }
                            })
                            .collect();
                        let s: u64 = w.iter().sum::<u64>().max(1);
                        let mut c: Vec<u64> = w.iter_mut().map(|x| *x * total / s).collect();
                        let rest = total - c.iter().sum::<u64>();
                        let j = (lcg(&mut seed) as usize) % items;
                        c[j] += rest;
                        let base = Table::from_counts(k, mu, &c).unwrap();
                        assert_eq!(base.counts(items), c);
                        let t = arrange(&base, items);
                        assert_eq!(t.counts(items), c);
                        for p in items..buckets {
                            assert_eq!(t.keep[p], 0);
                        }
                        assert!(t.keep.iter().all(|&x| u64::from(x) < cap));
                        let big = (0..items).max_by_key(|&j| (c[j], usize::MAX - j)).unwrap();
                        let feed = usize::try_from(c[big] / cap).unwrap().min(buckets - items);
                        for d in 0..feed {
                            assert_eq!(t.alt[buckets - 1 - d] as usize, big);
                        }
                    }
                }
            }
        }
    }
}
