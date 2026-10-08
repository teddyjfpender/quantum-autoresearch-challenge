//! PREPARE-side data and bounds. Release only.
use super::{lane_map, Params, Tweaks};
use crate::lanemap::LaneMap;
use crate::spec::sa::SaSpec;

fn pinned(id: &str) -> Box<dyn crate::spec::EncodingSpec> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::spec::load(root, id).unwrap()
}

#[test]
#[ignore = "pinned Li tables; run in release"]
fn aligned_alt_bit_keeps_every_count() {
    let boxed = pinned("li-sa-est-v1");
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let base = Params::for_spec(s);
    let p0 = Params {
        tw: Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4c"),
        outer: (base.outer.0, 8),
        inner: (base.inner.0, 8),
        ..base
    };
    let p1 = Params {
        tw: Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4ca"),
        ..p0
    };
    let (m0, m1) = (lane_map(s, p0).unwrap(), lane_map(s, p1).unwrap());
    for (i, (a, b)) in m0.inner.iter().zip(&m1.inner).enumerate().skip(s.n) {
        assert_eq!(a.counts(s.b + 1), b.counts(s.b + 1), "table {i}");
        let top = b.alt[63];
        assert_eq!(&b.alt[61..64], &[top; 3], "table {i}");
        for row in 18..38 {
            assert_eq!((b.alt[row] ^ top) & 32, 0, "table {i}, row {row}");
        }
    }
    assert_eq!(m0.rounding_error(s).unwrap(), m1.rounding_error(s).unwrap());
}

#[test]
#[ignore = "pinned Li estimated class; run in release"]
fn sparse_keep_alias_is_exact_estimated_class() {
    use crate::spec::rounding::RoundingClass;
    let boxed = pinned("li-sa-est-v1");
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let base = Params::for_spec(s);
    let p = Params {
        tw: Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4cb"),
        outer: (base.outer.0, 8),
        inner: (base.inner.0, 8),
        ..base
    };
    let map = lane_map(s, p).unwrap();
    let RoundingClass::EstimatedLow2025(params) = &s.rounding_class else {
        panic!("Li estimated spec needs its estimated class");
    };
    map.rounding_estimate(s, params).unwrap();
    let high_rows = [3, 4, 5, 18, 25, 31, 33, 38];
    for (q, t) in map.inner.iter().skip(s.n).enumerate() {
        assert_eq!(t.counts(s.b + 1).iter().sum::<u64>(), 1 << 14);
        assert_eq!(&t.alt[61..64], &[t.alt[63]; 3], "table {q}");
        assert!(
            t.keep
                .iter()
                .enumerate()
                .all(|(i, &k)| k < 128 || high_rows.contains(&i)),
            "table {q} has a high keep outside the sparse rows"
        );
    }
}

#[test]
fn joint_alias_small_table_counts_and_mutant() {
    use crate::lanemap::df_nested::Table;
    let table = Table {
        k: 2,
        mu: 2,
        keep: vec![1, 0, 3, 0],
        alt: vec![1, 2, 0, 1],
    };
    let mut enumerated = vec![0; 4];
    for bucket in 0..4 {
        for draw in 0..4 {
            let chosen = if draw < table.keep[bucket] {
                bucket
            } else {
                table.alt[bucket] as usize
            };
            enumerated[chosen] += 1;
        }
    }
    assert_eq!(table.counts(4), enumerated);
    let mut mutant = table.clone();
    mutant.alt[0] = 2;
    assert_ne!(mutant.counts(4), enumerated);
}

#[test]
#[ignore = "pinned Li estimated class; run in release"]
fn joint_alias_is_exact_estimated_class_and_mutant_fails() {
    use crate::spec::rounding::RoundingClass;
    let boxed = pinned("li-sa-est-v1");
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let base = Params::for_spec(s);
    let p = Params {
        tw: Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4cbf"),
        outer: (base.outer.0, 8),
        inner: (base.inner.0, 8),
        ..base
    };
    let mut map = lane_map(s, p).unwrap();
    let parent = lane_map(
        s,
        Params {
            tw: Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4cb"),
            ..p
        },
    )
    .unwrap();
    let RoundingClass::EstimatedLow2025(params) = &s.rounding_class else {
        panic!("Li estimated spec needs its estimated class");
    };
    map.rounding_estimate(s, params).unwrap();
    let high_rows = [3, 4, 5, 18, 24, 25, 31, 33, 34, 38];
    for (q, t) in map.inner.iter().skip(s.n).enumerate() {
        assert_eq!(t.counts(s.b + 1).iter().sum::<u64>(), 1 << 14);
        assert_eq!(
            t.counts(s.b + 1),
            parent.inner[s.n + q].counts(s.b + 1),
            "table {q} changed item counts"
        );
        assert_eq!(&t.alt[61..64], &[t.alt[63]; 3], "table {q}");
        assert!(
            t.keep
                .iter()
                .enumerate()
                .all(|(i, &k)| k < 128 || high_rows.contains(&i)),
            "table {q} has a high keep outside the joint rows"
        );
        for row in 18..38 {
            assert_eq!((t.alt[row] ^ t.alt[63]) & 32, 0, "table {q}, row {row}");
        }
    }
    assert_eq!(
        map.rounding_error(s).unwrap(),
        parent.rounding_error(s).unwrap()
    );
    let t = &mut map.inner[s.n];
    t.alt[0] = (t.alt[0] + 1) % (s.b as u32 + 1);
    assert!(map.rounding_estimate(s, params).is_err());
}

/// Dumps the alias tables and the weights they round (`PF_OUT` directory) for the pinned specs.
#[test]
#[ignore = "pinned specs; run in release"]
fn dump_tables() {
    use std::fmt::Write as _;
    let out = std::env::var("PF_OUT").unwrap();
    for id in ["li-sa-est-v1", "reiher-sa-est-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for mu in [8u32, 9] {
            let base = Params::for_spec(s);
            let p = Params {
                tw: Tweaks::parse("imchxgr"),
                outer: (base.outer.0, mu),
                inner: (base.inner.0, mu),
                ..base
            };
            let map = lane_map(s, p).unwrap();
            let mut t = String::new();
            writeln!(t, "N {} R {} B {} C {}", s.n, s.r, s.b, s.c).unwrap();
            let tb = |t: &mut String, name: &str, tab: &crate::lanemap::df_nested::Table| {
                writeln!(t, "{name} {} {}", tab.k, tab.mu).unwrap();
                writeln!(
                    t,
                    "{}",
                    tab.keep
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .unwrap();
                writeln!(
                    t,
                    "{}",
                    tab.alt
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                )
                .unwrap();
            };
            tb(&mut t, "outer", &map.outer);
            for (o, tab) in map.inner.iter().enumerate() {
                tb(&mut t, &format!("inner{o}"), tab);
            }
            writeln!(
                t,
                "e {}",
                s.e.iter()
                    .map(|x| format!("{x:e}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .unwrap();
            writeln!(
                t,
                "w {}",
                s.w.iter()
                    .map(|x| format!("{x:e}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .unwrap();
            writeln!(
                t,
                "wb {}",
                s.wb.iter()
                    .map(|x| format!("{x:e}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .unwrap();
            std::fs::write(format!("{out}/{id}-m{mu}.txt"), t).unwrap();
        }
    }
}

/// Lever `+`'s gadget (`itemhot::fan_rooted` with the offset read and erasure), exhaustively
/// over every item (in and out of range), inner index, root value, both item one-hot layouts and
/// three outcome seeds of random tables whose top rows equal a per-item word `A(item)` (with a
/// few deliberate exceptions, so some rows survive the offset): the read gives `root . word`,
/// the lane ends clean with the phase exactly `(-1)^(root . p(item, i))`, and the offset read
/// skips the zero rows. Mutants 100-104 are each caught.
mod offset_gadget {
    use crate::circuit::{Bit, Builder};
    use crate::walk::sa_low::itemhot::{
        erase, fan_rooted, phase_rooted, read_into_ext, rows_needed, write, ItemHot,
    };
    use crate::walk::sa_low::onehot::FAULT;
    use crate::walk::sa_low::qroam::Split;
    use crate::walk::shared::lookup::{word, Word};
    use crate::walk::shared::testsim::Sim;

    fn rnd(x: &mut u64) -> u64 {
        *x ^= *x << 13;
        *x ^= *x >> 7;
        *x ^= *x << 17;
        *x
    }

    /// `tab[item][i]`: `w` bits (top bit the phase); rows `i >= n_i - pad` equal the item's
    /// `A`, except item 1 (if any) at row `n_i - pad`, which differs in bit 0.
    fn table(seed: u64, items: usize, n_i: usize, w: usize, pad: usize) -> Vec<Vec<u64>> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..items)
            .map(|it| {
                let a = rnd(&mut x) % (1 << w);
                (0..n_i)
                    .map(|i| {
                        if i + pad >= n_i {
                            if it == 1 && i + pad == n_i {
                                a ^ 1
                            } else {
                                a
                            }
                        } else {
                            rnd(&mut x) % (1 << w)
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
    ) -> (u64, bool, u64) {
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
        let top = n_i - 1;
        let full = |it: u64, ii: u64| -> u64 {
            if in_range(it) {
                tab[(it - start) as usize][ii as usize]
            } else {
                0
            }
        };
        let wmask = (1u64 << (w - 1)) - 1;
        let a_of = |it: u64| -> Word {
            let mut x = word(w - 1);
            x[0] = full(it, top) & wmask;
            x
        };
        let p_of = |it: u64| full(it, top) >> (w - 1) & 1 == 1;
        let data = |it: u64, ii: u64| -> Word {
            let mut x = word(w - 1);
            x[0] = full(it, ii) & wmask;
            x
        };
        let ph = |it: u64, ii: u64| full(it, ii) >> (w - 1) & 1 == 1;
        let d_off = |it: u64, ii: u64| -> Word {
            let mut x = word(w - 1);
            x[0] = (full(it, ii) ^ full(it, top)) & wmask;
            x
        };
        let p_off = |it: u64, ii: u64| ph(it, ii) ^ p_of(it);
        let p_word = |it: u64, ii: u64| -> Word {
            let mut x = word(1);
            x[0] = u64::from(p_off(it, ii));
            x
        };
        FAULT.with(|c| c.set(fault));
        write(&mut b, &hot, &item);
        let out = b.alloc_n(w - 1);
        let rows = rows_needed(&hot, n_i, &[&d_off, &p_word]);
        read_into_ext(
            &mut b,
            &hot,
            &ireg,
            rows,
            &out,
            &d_off,
            Some(root),
            Some(&p_off),
        );
        fan_rooted(&mut b, &hot, root, &out, &a_of, Some(&p_of), &[]);
        let mid = b.ops().len();
        let m: Vec<Bit> = out.iter().map(|&q| b.hmr(q)).collect();
        let erows = rows_needed(&hot, n_i, &[&d_off]);
        phase_rooted(&mut b, &hot, &ireg, erows, &[(&m, &d_off)], Some(root));
        fan_rooted(&mut b, &hot, root, &[], &|_| word(1), None, &[(&m, &a_of)]);
        let _ = data;
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
        (got, clean, rows)
    }

    #[test]
    fn offset_read_and_erasure_are_exact() {
        for (n, start) in [(5usize, 3u64), (6, 0), (9, 7), (13, 2)] {
            for (n_i, pad) in [(8u64, 3usize), (6, 2), (8, 0)] {
                let w = 6;
                let tab = table(n as u64 * 17 + n_i, n, n_i as usize, w, pad);
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
                                    let (got, clean, rows) = lane(
                                        n, start, n_i, w, &tab, v, i, root_on, seed, 0, aligned,
                                    );
                                    assert_eq!(got, want, "n {n} item {v} i {i} root {root_on}");
                                    assert!(clean, "n {n} item {v} i {i} root {root_on}: dirty");
                                    // Rows past the deliberate exception are skipped.
                                    if pad > 1 {
                                        assert!(rows <= n_i - pad as u64 + 1, "rows {rows}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn offset_mutants_are_caught() {
        let (n, start, n_i, w, pad) = (9usize, 7u64, 8u64, 6usize, 3usize);
        let tab = table(5, n, n_i as usize, w, pad);
        for fault in [100u8, 101, 102, 103] {
            let mut caught = false;
            'all: for v in start..start + n as u64 {
                for i in 0..n_i {
                    for seed in 1..4 {
                        for aligned in [false, true] {
                            let want = tab[(v - start) as usize][i as usize] & 31;
                            let (got, clean, _) =
                                lane(n, start, n_i, w, &tab, v, i, true, seed, fault, aligned);
                            if got != want || !clean {
                                caught = true;
                                break 'all;
                            }
                        }
                    }
                }
            }
            assert!(caught, "fault {fault} must break a lane");
        }
    }
}

/// GF(2) rank of bit-vector rows (`u128` limbs, any width).
fn rank_rows(rows: &[Vec<u64>]) -> usize {
    let mut basis: Vec<(usize, Vec<u64>)> = Vec::new();
    for row in rows {
        let mut r = row.clone();
        for (pc, bv) in &basis {
            if r[pc / 64] >> (pc % 64) & 1 == 1 {
                r.iter_mut().zip(bv).for_each(|(x, y)| *x ^= y);
            }
        }
        if let Some(pc) = (0..r.len() * 64).find(|&c| r[c / 64] >> (c % 64) & 1 == 1) {
            for (_, bv) in &mut basis {
                if bv[pc / 64] >> (pc % 64) & 1 == 1 {
                    bv.iter_mut().zip(&r).for_each(|(x, y)| *x ^= y);
                }
            }
            basis.push((pc, r));
        }
    }
    basis.len()
}

/// The PREPARE side's floors, per spec and keep split (`SA_SPECS`, `SA_MU`), for
/// the inner alias read (the largest PREPARE cost) and its erasure.
///
/// - **Row-space bound (theorem in the affine-control family).** A
///   lookup read from a materialized exclusive code (item one-hot `h`, two groups, iterated over
///   the inner index; or the paired lookup's slot one-hot, iterated over groups) whose Toffolis
///   take affine controls adds, per output bit, at most two row vectors per Toffoli, so bit `k`
///   costs at least `ceil(rank_k / 2)` where `rank_k` is the GF(2) rank of its (row x slot)
///   matrix. Printed with the construction's count (one Toffoli per nonzero row pair).
/// - **Counting floors (any circuit; heuristic for these tables, theorem for random tables of
///   the same shape).** With `n` input bits, `Q'` live non-system wires and free Cliffords, a
///   circuit with `t` Toffolis is fixed by at most `t (3 Q' + O(log))` choice bits plus the
///   outputs' `w (n + t + 1)`; a random `L`-cell, `w`-bit table needs `t >= w (L - n - 1) /
///   (3 Q' + w)`. A phase (erasure) of a random `L`-cell function needs
///   `2 t (n + t) + (n + t + 1)^2 / 2 >= L`.
/// - **Degree-2 erasure floor (theorem in the one-hot family).** A phase table built from two
///   materialized one-hots of `S1`, `S2` slots (all phases then `CZ`s) needs `S1 S2 >= L` and
///   `S1 + S2 - 2` ANDs: at least `2 sqrt(L) - 2`.
#[test]
#[ignore = "pinned specs; run in release"]
#[allow(clippy::too_many_lines, clippy::needless_range_loop)]
fn prep_floor() {
    use super::tables::SaTables;
    let mus: Vec<(u32, u32)> = std::env::var("SA_MU").map_or_else(
        |_| vec![(9, 9), (8, 8)],
        |v| {
            let x: Vec<u32> = v.split(',').map(|y| y.parse().unwrap()).collect();
            vec![(x[0], x[1])]
        },
    );
    let qs: Vec<usize> = std::env::var("SA_QS")
        .unwrap_or_else(|_| "253,316,433,492".into())
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    for id in std::env::var("SA_SPECS")
        .unwrap_or_else(|_| "li-sa-est-v1,reiher-sa-est-v1".into())
        .split(',')
    {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for &mu in &mus {
            for pad in [false, true] {
                let base = Params::for_spec(s);
                let tw = if pad { "imchxgrdkyHt+" } else { "imchxgrdkyHt" };
                let p = Params {
                    tw: Tweaks::parse(tw),
                    outer: (base.outer.0, mu.0),
                    inner: (base.inner.0, mu.1),
                    ..base
                };
                let map = lane_map(s, p).unwrap();
                let t = SaTables::with_items(s, &map, true);
                let k_i = map.inner[0].k as usize;
                let m = mu.1 as usize;
                let n_i = 1usize << k_i;
                let items = s.r * s.c;
                let bb = s.b as u64;
                let bit = |w: &[u64], at: usize| w[at / 64] >> (at % 64) & 1;
                // Lever t's word `keep | delta | alt b` of square item q at inner index i.
                let tword = |q: usize, i: usize| -> u64 {
                    let wd = t.item_inner_word(((s.n + q) << k_i | i) as u64);
                    let keep = (0..m).fold(0u64, |a, j| a | bit(&wd, j) << j);
                    let own = bit(&wd, m);
                    let alt = (0..k_i).fold(0u64, |a, j| a | bit(&wd, m + 1 + j) << j);
                    let altf = bit(&wd, m + 1 + k_i);
                    let delta = if keep == 0 || i as u64 > bb {
                        0
                    } else {
                        own ^ altf
                    };
                    keep | delta << m | alt << (m + 1)
                };
                let wbits = m + 1 + k_i;
                let words: Vec<Vec<u64>> = (0..items)
                    .map(|q| {
                        let a = if pad { tword(q, n_i - 1) } else { 0 };
                        (0..n_i).map(|i| tword(q, i) ^ a).collect()
                    })
                    .collect();
                // H path (aligned two-group item one-hot: slot q >> 1, group q & 1).
                let e1 = items.div_ceil(2);
                let limbs = e1.div_ceil(64);
                let (mut bound_h, mut built_h) = (0usize, 0usize);
                for kb in 0..wbits {
                    let mut rows: Vec<Vec<u64>> = Vec::new();
                    for i in 0..n_i {
                        let mut pairs_nonzero = false;
                        for g in 0..2 {
                            let mut v = vec![0u64; limbs];
                            for sl in 0..e1 {
                                let q = 2 * sl + g;
                                if q < items && words[q][i] >> kb & 1 == 1 {
                                    v[sl / 64] |= 1 << (sl % 64);
                                    pairs_nonzero = true;
                                }
                            }
                            rows.push(v);
                        }
                        built_h += usize::from(pairs_nonzero);
                    }
                    bound_h += rank_rows(&rows).div_ceil(2);
                }
                let l_cells = items * n_i;
                let n_in = k_i + (64 - ((s.n + items) as u64).leading_zeros()) as usize;
                let counting = |q_live: usize| -> f64 {
                    (wbits * (l_cells - n_in - 1)) as f64 / (3 * q_live + wbits) as f64
                };
                let erase_count = (0..)
                    .find(|&tt: &usize| {
                        2 * tt * (n_in + tt) + (n_in + tt + 1).pow(2) / 2 >= l_cells
                    })
                    .unwrap();
                println!(
                    "PREPFLOOR {id} mu {mu:?} {}: L {l_cells} cells, w {wbits}, n_in {n_in}; H read per copy: row-space bound {bound_h} (+ {n_i} iteration + {n_i} gates), built {built_h} pairs; erasure: degree-2 floor {:.0}, counting {erase_count}",
                    if pad { "with +" } else { "plain" },
                    2.0 * (l_cells as f64).sqrt() - 2.0
                );
                if !pad {
                    // G path: slot = x mod 2^8 (Li) / 2^7 (Reiher) of x = item 2^k_i + i.
                    let sb = if k_i >= 6 { 8 } else { 7 };
                    let lam = 1usize << sb;
                    let (start, limit) = (s.n << k_i, (s.n + items) << k_i);
                    let (g0, g1) = (start / lam, limit.div_ceil(lam));
                    let (mut bound_g, mut built_g) = (0usize, 0usize);
                    for kb in 0..wbits {
                        let mut rows: Vec<Vec<u64>> = Vec::new();
                        let mut nz = std::collections::BTreeSet::new();
                        for g in g0..g1 {
                            let mut v = vec![0u64; lam / 64];
                            for sl in 0..lam {
                                let x = g * lam + sl;
                                if (start..limit).contains(&x) {
                                    let (q, i) = ((x >> k_i) - s.n, x & (n_i - 1));
                                    if words[q][i] >> kb & 1 == 1 {
                                        v[sl / 64] |= 1 << (sl % 64);
                                        nz.insert(g / 2);
                                    }
                                }
                            }
                            rows.push(v);
                        }
                        bound_g += rank_rows(&rows).div_ceil(2);
                        built_g += nz.len();
                    }
                    println!(
                        "  G read (slot one-hot 2^{sb}) per copy: row-space bound {bound_g} (+ {} slot ANDs + {} iteration), built {built_g} pairs",
                        lam - 1,
                        g1 - g0
                    );
                    for &q in &qs {
                        let q_live = q.saturating_sub(2 * s.n + s.beta as usize);
                        println!(
                            "  Q {q}: counting read floor {:.0} per copy (Q' {q_live})",
                            counting(q_live)
                        );
                    }
                }
            }
        }
    }
}

/// The read and erasure bounds of the **dual one-hot bilinear
/// PREPARE** (DOB, analysis only; nothing is built). The item one-hot is unsplit (one slot per
/// square item, `G = 1`), so bit `k` of the inner word is the bilinear form `iota^T W_k h` of the
/// item one-hot `h` and the inner-index one-hot `iota`. With both one-hots materialized:
///
/// - a Toffoli on `(a.h ^ b'.iota)(b.iota ^ a'.h)` adds two rank-1 terms to one bit (the products
///   of two parities of one one-hot are linear), so bit `k` costs `ceil(rank_k / 2)` with
///   `rank_k` the GF(2) rank of the `(i x item)` matrix (`dob read bound`);
/// - without `iota`, pairing the two leaves `i_0 = 0, 1` of one parent of the unary iteration
///   (`u1 = p AND i_0`, `u0 = p ^ u1`) costs one Toffoli per (parent, bit) with a nonzero pair
///   (`dob i0-paired read`);
/// - the erasure's phase `(-1)^(iota^T (sum_k m_k W_k) h)` is a product of `CZ`s between parities:
///   Cliffords only, once both one-hots are live.
///
/// Prints those counts and the two-group read's (the built `H` read's row-space bound) for
/// comparison. `SA_SPECS`, `SA_MU` as `prep_floor`.
#[test]
#[ignore = "pinned specs; run in release"]
#[allow(clippy::needless_range_loop)]
fn dob_bound() {
    use super::tables::SaTables;
    let mu: u32 = std::env::var("SA_MU")
        .ok()
        .and_then(|v| v.split(',').next().and_then(|x| x.parse().ok()))
        .unwrap_or(9);
    for id in std::env::var("SA_SPECS")
        .unwrap_or_else(|_| "reiher-sa-est-v1,li-sa-est-v1".into())
        .split(',')
    {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for pad in [false, true] {
            let base = Params::for_spec(s);
            let tw = if pad { "imchxgrdkyHt+" } else { "imchxgrdkyHt" };
            let p = Params {
                tw: Tweaks::parse(tw),
                outer: (base.outer.0, mu),
                inner: (base.inner.0, mu),
                ..base
            };
            let map = lane_map(s, p).unwrap();
            let t = SaTables::with_items(s, &map, true);
            let k_i = map.inner[0].k as usize;
            let m = mu as usize;
            let n_i = 1usize << k_i;
            let items = s.r * s.c;
            let bb = s.b as u64;
            let bit = |w: &[u64], at: usize| w[at / 64] >> (at % 64) & 1;
            let tword = |q: usize, i: usize| -> u64 {
                let wd = t.item_inner_word(((s.n + q) << k_i | i) as u64);
                let keep = (0..m).fold(0u64, |a, j| a | bit(&wd, j) << j);
                let own = bit(&wd, m);
                let alt = (0..k_i).fold(0u64, |a, j| a | bit(&wd, m + 1 + j) << j);
                let altf = bit(&wd, m + 1 + k_i);
                let delta = if keep == 0 || i as u64 > bb {
                    0
                } else {
                    own ^ altf
                };
                keep | delta << m | alt << (m + 1)
            };
            let wbits = m + 1 + k_i;
            let words: Vec<Vec<u64>> = (0..items)
                .map(|q| {
                    let a = if pad { tword(q, n_i - 1) } else { 0 };
                    (0..n_i).map(|i| tword(q, i) ^ a).collect()
                })
                .collect();
            let limbs = items.div_ceil(64);
            let (mut dob, mut paired, mut two_group) = (0usize, 0usize, 0usize);
            let e1 = items.div_ceil(2);
            for kb in 0..wbits {
                // Unsplit: one row per inner index over the `items` slots.
                let rows: Vec<Vec<u64>> = (0..n_i)
                    .map(|i| {
                        let mut v = vec![0u64; limbs];
                        for q in 0..items {
                            if words[q][i] >> kb & 1 == 1 {
                                v[q / 64] |= 1 << (q % 64);
                            }
                        }
                        v
                    })
                    .collect();
                dob += rank_rows(&rows).div_ceil(2);
                paired += (0..n_i / 2)
                    .filter(|&pp| {
                        rows[2 * pp]
                            .iter()
                            .chain(&rows[2 * pp + 1])
                            .any(|&x| x != 0)
                    })
                    .count();
                // The built two-group read's rows `(g, i)` over `e1` slots.
                let mut rows2: Vec<Vec<u64>> = Vec::new();
                for i in 0..n_i {
                    for g in 0..2 {
                        let mut v = vec![0u64; e1.div_ceil(64)];
                        for sl in 0..e1 {
                            let q = 2 * sl + g;
                            if q < items && words[q][i] >> kb & 1 == 1 {
                                v[sl / 64] |= 1 << (sl % 64);
                            }
                        }
                        rows2.push(v);
                    }
                }
                two_group += rank_rows(&rows2).div_ceil(2);
            }
            println!(
                "DOB {id} mu {mu} {}: items {items}, n_i {n_i}, w {wbits}; two-group (built H) read bound {two_group}; dob read bound (both one-hots) {dob}; dob i0-paired read {paired} (+ {} parents' ANDs); unsplit one-hot write {} ANDs vs two-group {}",
                if pad { "with +" } else { "plain" },
                n_i / 2,
                items - 1,
                e1 - 1
            );
        }
    }
}
