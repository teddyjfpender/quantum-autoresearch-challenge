//! Analyses of the rotation delivery on the pinned specs.
//! Release only, `#[ignore]`d.
use crate::spec::sa::SaSpec;

fn spec(id: &str) -> Box<dyn crate::spec::EncodingSpec> {
    super::tests_combo::pinned(id)
}

/// GF(2) rank of bit vectors (each a `Vec<u64>`).
pub(super) fn rank(rows: &[Vec<u64>]) -> usize {
    let mut basis: Vec<(usize, Vec<u64>)> = Vec::new();
    for row in rows {
        let mut r = row.clone();
        for (pc, bv) in &basis {
            if r[pc / 64] >> (pc % 64) & 1 == 1 {
                for (x, y) in r.iter_mut().zip(bv) {
                    *x ^= y;
                }
            }
        }
        if let Some(pc) = (0..r.len() * 64).find(|&c| r[c / 64] >> (c % 64) & 1 == 1) {
            basis.push((pc, r));
        }
    }
    basis.len()
}

/// Per transition and group pair of a plain `G`-group split one-hot (leaf `e` at slot `e mod
/// e1`, group `e div e1`): the bits whose pair correction is nonzero (what `z` pays) against the
/// GF(2) rank of those bits' pair vectors (what a correction shared across bits would pay).
/// `SA_SPECS`, `SA_G` (default 5), `SA_SIGN` (1: lever `s`'s data).
#[test]
#[ignore = "pinned specs; run in release"]
fn transition_rank() {
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1".into());
    let g: usize = std::env::var("SA_G").ok().map_or(5, |v| v.parse().unwrap());
    let sign = std::env::var("SA_SIGN").is_ok_and(|v| v == "1");
    for id in ids.split(',') {
        let boxed = spec(id);
        let s0: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let normed = super::signnorm::sign_normalized(s0);
        let s = if sign { &normed } else { s0 };
        let nets: Vec<_> = s.nets.iter().chain(&s.e_nets).collect();
        let e = nets.len();
        let e1 = e.div_ceil(g);
        let rots = s.n - 1;
        let w = usize::from(s.beta);
        let ang = |l: usize, j: usize| u64::from(nets[l].rotations[j].2);
        let leaf = |grp: usize, c: usize| (grp * e1 + c < e).then_some(grp * e1 + c);
        let words = (2 * e1).div_ceil(64);
        let (mut paid, mut ranked) = (0usize, 0usize);
        // Transitions of one pass: load of the last rotation, then down to rotation 0.
        let mut steps: Vec<(Option<usize>, usize)> = vec![(None, rots - 1)];
        steps.extend((0..rots - 1).rev().map(|j| (Some(j + 1), j)));
        for &(from, to) in &steps {
            let delta = |l: usize| from.map_or(0, |f| ang(l, f)) ^ ang(l, to);
            let d = |grp: usize, c: usize| -> u64 {
                match (leaf(0, c), leaf(grp, c)) {
                    (_, None) => 0,
                    (Some(l0), Some(lg)) => delta(l0) ^ delta(lg),
                    (None, Some(lg)) => delta(lg),
                }
            };
            for p in 0..(g - 1) / 2 {
                let (a, c) = (2 * p + 1, 2 * p + 2);
                let vecs: Vec<Vec<u64>> = (0..w)
                    .map(|k| {
                        let mut v = vec![0u64; words];
                        for slot in 0..e1 {
                            if d(a, slot) >> k & 1 == 1 {
                                v[slot / 64] |= 1 << (slot % 64);
                            }
                            if d(c, slot) >> k & 1 == 1 {
                                let x = e1 + slot;
                                v[x / 64] |= 1 << (x % 64);
                            }
                        }
                        v
                    })
                    .collect();
                paid += vecs.iter().filter(|v| v.iter().any(|&x| x != 0)).count();
                ranked += rank(&vecs);
            }
        }
        println!(
            "RANK {id} G {g} sign {sign}: per pass, pair corrections {paid} (per-bit), {ranked} (rank-shared), saving {}",
            paid - ranked
        );
    }
}

/// Distinct networks (as angle sequences) among the leaves, raw and sign-normalised, and how many
/// leaves are unreachable (outer weight 0): leaves sharing a network, or never reached, could
/// share a one-hot slot. `SA_SPECS`.
#[test]
#[ignore = "pinned specs; run in release"]
fn duplicate_networks() {
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1,reiher-sa-est-v1".into());
    for id in ids.split(',') {
        let boxed = spec(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let normed = super::signnorm::sign_normalized(s);
        let count = |sp: &SaSpec| {
            let mut seen = std::collections::BTreeSet::new();
            for n in sp.nets.iter().chain(&sp.e_nets) {
                seen.insert(n.rotations.iter().map(|r| r.2).collect::<Vec<_>>());
            }
            seen.len()
        };
        let total = s.nets.len() + s.e_nets.len();
        let zero_w = s.w.iter().filter(|&&x| x == 0.0).count();
        let zero_e = s.e.iter().filter(|&&x| x == 0.0).count();
        println!(
            "DUP {id}: leaves {total}, distinct networks {} raw, {} sign-normalised; zero square weights {zero_w}, zero one-body eigenvalues {zero_e}",
            count(s),
            count(&normed)
        );
    }
}

/// The RPREP one-hot's slot count under lever `C`'s class layout against `ceil(E / G)`, per
/// group count (`SA_SPECS`, `SA_MU`, `SA_TWEAKS` for the tables' layout).
#[test]
#[ignore = "pinned specs; run in release"]
fn class_slots() {
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1,reiher-sa-est-v1".into());
    for id in ids.split(',') {
        let boxed = spec(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = super::tests_combo::params(s, "imchxgrdky5zabCHVIXJ");
        let map = super::lane_map(s, p).unwrap();
        let t = super::tables::SaTables::with_items(s, &map, true);
        let e = super::onehot::leaves(&t);
        for g in 2..=9usize {
            let (_, e1) = super::onehot::class_layout(&t, g, false);
            let (_, e1o) = super::onehot::class_layout_with(&t, g, true, false);
            let plan = super::onehot::class_plan(&t, g, true, false);
            println!(
                "SLOTS {id} G {g}: class layout {e1} slots (s_ob {}), packed (o) {e1o} slots at depths {:?} in {} classes, ceil(E/G) = {}",
                super::onehot::best_s_ob(&t, g),
                super::onehot::best_pack(&t, g),
                plan.defs.len(),
                e.div_ceil(g)
            );
        }
    }
}

/// Lever `o`'s options per split depth: slots and the write + erase Toffolis per copy of the class
/// one-hot, `2 V - rows + e1 - 2 C` (sub-row splits, class expansions, the unfold), against the
/// consecutive layout. `SA_SPECS`, `SA_G` (comma list).
#[test]
#[ignore = "pinned specs; run in release"]
fn pack_options() {
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1".into());
    let gs = std::env::var("SA_G").unwrap_or_else(|_| "4,6,7".into());
    for id in ids.split(',') {
        let boxed = spec(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = super::tests_combo::params(s, "imchxgrdky5zabCHVIXJ");
        let map = super::lane_map(s, p).unwrap();
        let t = super::tables::SaTables::with_items(s, &map, true);
        let rows = t.rows() as usize;
        for g in gs.split(',').map(|x| x.parse::<usize>().unwrap()) {
            let base = super::onehot::class_plan(&t, g, false, false);
            let cost =
                |vr: usize, c: usize, e1: usize| super::onehot::class_toffolis(vr, rows, c, e1);
            println!(
                "PACK {id} G {g} consecutive: e1 {} classes {} T/copy {}",
                base.e1,
                base.defs.len(),
                cost(base.vr.len(), base.defs.len(), base.e1)
            );
            let mut opts = Vec::new();
            for s_sq in 0..=t.k_i {
                for s_ob in 0..=t.k_i {
                    let vr = super::onehot::vrows_sq(&t, s_sq, s_ob);
                    let plan_vr = vr.len();
                    let (defs, e1) = super::onehot::defs_packed_pub(&vr, g);
                    opts.push((e1, cost(plan_vr, defs.len(), e1), s_sq, s_ob, defs.len()));
                }
            }
            opts.sort_unstable();
            let mut front: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
            for o in opts {
                if front.last().is_none_or(|f| o.1 < f.1) {
                    front.push(o);
                }
            }
            for (e1, tc, sq, ob, c) in front {
                println!(
                    "PACK {id} G {g} packed depths ({sq}, {ob}): e1 {e1} classes {c} T/copy {tc}"
                );
            }
        }
    }
}
