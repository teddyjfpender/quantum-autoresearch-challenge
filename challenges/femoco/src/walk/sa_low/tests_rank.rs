//! Analysis dumps for rank-scheduled angle delivery.
//!
//! `dump_rank_layout` writes the split one-hot layout of a bundle and every leaf's stored angle
//! words **as the circuit delivers them** (sign-normalised when the bundle has `s`), for offline
//! schedule analysis. No circuit is built. `SA_SPECS`, `SA_TWEAKS`, `SA_MU` as in `tests_combo`;
//! `SA_DUMP` the output path.
use super::tables::SaTables;
use super::{lane_map, onehot};
use crate::lanemap::LaneMap;
use crate::spec::sa::SaSpec;
use std::fmt::Write as _;

#[test]
#[ignore = "pinned specs; analysis dump"]
fn dump_rank_layout() {
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-est-v1".into());
    let tw = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky3zabCHKVIDXtNBWs".into());
    let out = std::env::var("SA_DUMP").expect("SA_DUMP path");
    let boxed = super::tests_combo::pinned(&id);
    let s0: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let p = super::tests_combo::params(s0, &tw);
    let map = lane_map(s0, p).unwrap();
    let normed;
    let s = if p.tw.sign_norm {
        normed = super::signnorm::sign_normalized(s0);
        &normed
    } else {
        s0
    };
    let t = SaTables::with_items(s, &map, p.tw.derive_id);
    let g = p.tw.hot_groups as usize;
    let (fold, e1) = onehot::class_layout(&t, g.max(1), p.tw.class_mixed);
    let n = onehot::leaves(&t);
    // Reachable leaves (as tests_floor): a one-body row with outer lanes, a square (r, b) with an
    // outer item (r, c) that has lanes and an inner count for b.
    let (nn, r, bb, c) = (s.n, s.r, s.b, s.c);
    let outer = map.outer.counts(nn + r * c);
    let mut reach = vec![false; n];
    for rr in 0..r {
        for b in 0..bb {
            reach[rr * bb + b] = (0..c).any(|cc| {
                let item = nn + rr * c + cc;
                outer[item] > 0 && map.inner[item].counts(bb + 1)[b] > 0
            });
        }
    }
    for k in 0..nn {
        reach[r * bb + k] = outer[k] > 0;
    }
    let mut txt = String::new();
    writeln!(
        txt,
        "N {} R {} B {} E {} e1 {} G {} widths {:?}",
        s.n, s.r, s.b, n, e1, g, t.widths
    )
    .unwrap();
    for gi in 0..g.max(1) {
        for cs in 0..e1 {
            if let Some(e) = fold.leaf_at[gi][cs] {
                let angs: Vec<String> = (0..t.widths.len())
                    .map(|j| onehot::angle(&t, e, j).to_string())
                    .collect();
                writeln!(
                    txt,
                    "L {gi} {cs} {e} {} {}",
                    u8::from(reach[e]),
                    angs.join(" ")
                )
                .unwrap();
            }
        }
    }
    std::fs::write(out, txt).unwrap();
}

/// Counts the op stream's Toffolis: unconditioned, and inside condition blocks (for checking a
/// static ledger against the harness). `SA_SPECS`, `SA_TWEAKS`, `SA_MU`, `SA_INNER_A`,
/// `SA_OUTER_A` as in `tests_combo`.
#[test]
#[ignore = "pinned specs; analysis"]
fn toffoli_census() {
    use crate::circuit::{OperationType as K, NONE};
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-est-v1".into());
    for tw in std::env::var("SA_TWEAKS")
        .unwrap_or_else(|_| "imchxgrdky3zabCHKVIDXtNBWs".into())
        .split(',')
    {
        let boxed = super::tests_combo::pinned(&id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = super::tests_combo::params(s, tw);
        let bt = super::tests_combo::build(s, p);
        let (mut plain, mut cond, mut depth) = (0u64, 0u64, 0usize);
        let mut by_stage: Vec<(u64, u64)> = vec![(0, 0); bt.led.rows.len() + 1];
        for (i, op) in bt.ops.iter().enumerate() {
            match op.kind {
                K::PushCondition => depth += 1,
                K::PopCondition => depth -= 1,
                K::CCX | K::CCZ => {
                    let r = bt.led.ends.partition_point(|&e| e <= i);
                    if depth == 0 && op.c_condition == NONE {
                        plain += 1;
                        by_stage[r].0 += 1;
                    } else {
                        cond += 1;
                        by_stage[r].1 += 1;
                    }
                }
                _ => {}
            }
        }
        println!(
            "CENSUS {id} {tw}: ops {} plain CCX/CCZ {plain} conditioned {cond}",
            bt.ops.len()
        );
        for (r, (a, c)) in by_stage.iter().enumerate() {
            if *a + *c > 0 {
                let name = bt.led.rows.get(r).map_or("(after)", |x| x.0.as_str());
                println!("  {a:>7} plain {c:>6} cond  {name}");
            }
        }
    }
}

/// Lever `q`'s plan on a pinned spec, per budget: the window plan's buys per copy, the delivery
/// model's segmented lower bound, `D'` and the number of windows. `SA_SPECS`, `SA_TWEAKS` (one
/// bundle with a split one-hot and `C`), `SA_MU`; `SA_BUDGETS` a comma list.
#[test]
#[ignore = "pinned specs; analysis"]
fn rank_plan_table() {
    use crate::circuit::Builder;
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-est-v1".into());
    let tw = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky3zabCHKVIDXtNBWs".into());
    let budgets: Vec<usize> = std::env::var("SA_BUDGETS")
        .unwrap_or_else(|_| "15,45,75,90,105,120,135,150,165,180,195,210".into())
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    let boxed = super::tests_combo::pinned(&id);
    let s0: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let p = super::tests_combo::params(s0, &tw);
    let map = lane_map(s0, p).unwrap();
    let normed;
    let s = if p.tw.sign_norm {
        normed = super::signnorm::sign_normalized(s0);
        &normed
    } else {
        s0
    };
    let t = SaTables::with_items(s, &map, p.tw.derive_id);
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let lo = b.alloc_n(t.k_i);
    let hi = b.alloc_n(t.h);
    let start = b.ops().len();
    let hot = onehot::write_classes_with(
        &mut b,
        &t,
        &lo,
        &hi,
        p.tw.hot_groups,
        p.tw.class_pack,
        p.tw.class_mixed,
    );
    let inputs: Vec<_> = lo.iter().chain(hi.iter()).copied().collect();
    let (kb, sp) = (lo.len(), t.spec);
    let valid = |a: u64| {
        let (l, h) = (a & ((1 << kb) - 1), a >> kb);
        onehot::leaf_of(&t, h << t.k_i | l).is_some() || (h < sp.r as u64 && l == sp.b as u64)
    };
    let reach = super::rankdel::reachable(&b.ops()[start..], &inputs, &valid, &hot);
    let lay = super::rankdel::Layout::of(&hot, onehot::leaves(&t), reach.as_deref());
    let angle = |e: usize, j: usize| onehot::angle(&t, e, j);
    let rots = t.modes.len();
    let width = onehot::register_bits(&t);
    for &bud in &budgets {
        let pl = super::rankdel::plan(&lay, &angle, rots, width, bud);
        let lb = super::rankdel::model_bound(&lay, &angle, rots, width, bud);
        let bel = super::rankdel::belady_buys(&lay, &angle, rots, width, bud);
        // The linear-factor bound (Toffolis per copy, any G).
        let lf = super::rankdel::lf_bound(&lay, &angle, rots, width, bud);
        // SA_ROLLOUT=<tries>[,<park>]: mode 2's buys against mode 1's.
        if let Ok(v) = std::env::var("SA_ROLLOUT") {
            let x: Vec<usize> = v.split(',').map(|y| y.parse().unwrap()).collect();
            let t0 = std::time::Instant::now();
            let (b1, b2) = super::rankdel::rollout_buys(
                &lay,
                &angle,
                rots,
                width,
                bud,
                x.get(1).copied().unwrap_or(0),
                x[0],
            );
            println!(
                "ROLLOUT {id} b {bud} park {:?}: mode 1 {b1} mode 2 {b2} ({:.1} s)",
                x.get(1),
                t0.elapsed().as_secs_f64()
            );
        }
        println!(
            "RANKPLAN {id} {tw} b {bud}: buys/copy {} (Belady {bel}) model LB {lb} LF bound {lf} D' {} windows {} max dim {}",
            pl.buys,
            pl.dprime,
            pl.starts.len(),
            pl.max_dim
        );
    }
}

/// The inner alias tables' lane counts per index (one line per square item:
/// `item keep... | alt... | counts...`), for offline alias-structure analysis. `SA_SPECS`,
/// `SA_TWEAKS`, `SA_MU`; `SA_DUMP` the output path.
#[test]
#[ignore = "pinned specs; analysis dump"]
fn dump_inner_tables() {
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-est-v1".into());
    let tw = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky3zabCHKVIDXtNBWs+".into());
    let out = std::env::var("SA_DUMP").expect("SA_DUMP path");
    let boxed = super::tests_combo::pinned(&id);
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let p = super::tests_combo::params(s, &tw);
    let map = lane_map(s, p).unwrap();
    let mut txt = String::new();
    writeln!(txt, "N {} R {} B {} C {}", s.n, s.r, s.b, s.c).unwrap();
    for (item, t) in map.inner.iter().enumerate().skip(s.n) {
        let n = t.counts(1 << t.k);
        let f = |v: &[u32]| {
            v.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        };
        let c: Vec<u32> = n.iter().map(|&x| x as u32).collect();
        writeln!(txt, "{item} {} | {} | {}", f(&t.keep), f(&t.alt), f(&c)).unwrap();
    }
    std::fs::write(out, txt).unwrap();
}
