//! Analysis and gadget tests for the `.` extension levers on the Li spec.
use super::tests_combo::{params, pinned};
use super::{lane_map, Params};
use crate::spec::sa::SaSpec;

fn li() -> (Box<dyn crate::spec::EncodingSpec>, String) {
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1".into());
    (pinned(&id), id)
}

/// Donor feasibility for the padding-slot merge: per square inner table, the largest count and
/// the second largest (lanes), against 4 and 2 padding buckets' worth.
#[test]
#[ignore = "pinned specs; run in release"]
fn donor_census() {
    let (boxed, id) = li();
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let tw =
        std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky4zabCAXZJtRNOBWMenFsY".into());
    let p: Params = params(s, &tw);
    let map = lane_map(s, p).unwrap();
    let cap = 1u64 << p.inner.1;
    let items = s.b + 1;
    let mut ok4 = 0;
    let mut ok42 = 0;
    let mut ok6 = 0;
    let mut minmax = u64::MAX;
    for t in map.inner.iter().skip(s.n) {
        let mut c = t.counts(items);
        c.sort_unstable_by(|a, b| b.cmp(a));
        minmax = minmax.min(c[0]);
        if c[0] >= 4 * cap {
            ok4 += 1;
        }
        if c[0] >= 6 * cap {
            ok6 += 1;
        }
        if c[0] >= 4 * cap && (c[0] >= 6 * cap || c[1] >= 2 * cap) {
            ok42 += 1;
        }
    }
    println!(
        "DONOR {id}: {} tables; largest >= 4 buckets: {ok4}; 4+2 split: {ok42}; one donor for 6: {ok6}; min largest {minmax} (cap {cap})",
        map.inner.len() - s.n
    );
}

/// The class plan of lever `C` with and without lever `.g` on a pinned spec: virtual rows,
/// classes (members, base, width), grafts and the slot count.
#[test]
#[ignore = "pinned specs; run in release"]
fn graft_plan() {
    let (boxed, id) = li();
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let tw =
        std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky4zabCAXZJtRNOBWMenFsY".into());
    let p: Params = params(s, &tw);
    let map = lane_map(s, p).unwrap();
    let t = super::tables::SaTables::with_items(s, &map, p.tw.derive_id);
    for graft in [false, true] {
        let plan = super::onehot::class_plan_g(
            &t,
            usize::from(p.tw.hot_groups),
            false,
            p.tw.class_mixed,
            graft,
        );
        println!("PLAN {id} graft {graft}: e1 {}", plan.e1);
        for d in &plan.defs {
            let m: Vec<String> = d
                .members
                .iter()
                .map(|&v| format!("{}:{}/{}", plan.vr[v].hv, plan.vr[v].sub, plan.vr[v].len))
                .collect();
            println!("  class base {} width {} members {:?}", d.base, d.width, m);
        }
        for g in &plan.grafts {
            println!("  graft {:?} vrow {:?}", g, plan.vr[g.v]);
        }
    }
}

/// Lever `.g`'s depth options on a pinned spec: for every forced one-body depth vector, the slot
/// count and graft count of the grafted plan (the trade-off the cost key chooses from).
#[test]
#[ignore = "pinned specs; run in release"]
fn graft_options() {
    let (boxed, id) = li();
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let tw =
        std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky4zabCAXZJtRNOBWMenFsY".into());
    let p: Params = params(s, &tw);
    let map = lane_map(s, p).unwrap();
    let t = super::tables::SaTables::with_items(s, &map, p.tw.derive_id);
    let n_ob = t.rows() as usize - s.r;
    let radix = t.k_i + 1;
    let mut seen = std::collections::BTreeMap::new();
    for code in 0..radix.pow(n_ob as u32) {
        let d: Vec<usize> = (0..n_ob)
            .map(|i| code / radix.pow(i as u32) % radix)
            .collect();
        super::onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = Some(d.clone()));
        let plan = super::onehot::class_plan_g(
            &t,
            usize::from(p.tw.hot_groups),
            false,
            p.tw.class_mixed,
            true,
        );
        super::onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = None);
        let e = seen.entry(plan.e1).or_insert((usize::MAX, d.clone()));
        if plan.grafts.len() < e.0 {
            *e = (plan.grafts.len(), d);
        }
    }
    for (e1, (g, d)) in seen {
        println!("OPT {id}: e1 {e1} grafts {g} depths {d:?}");
    }
}

/// Lever `q<b>_1.<c>` and the `.` extension levers parse together;
/// a `.` followed by a digit is the Majorana budget, the first `.` followed by a letter starts
/// the extension levers.
#[test]
fn combined_lever_syntax() {
    use super::Tweaks;
    let t = Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13");
    assert_eq!((t.rank_hold, t.rank_mode, t.rank_park), (16, 1, 13));
    assert!(!t.erase_pairs && !t.graft && !t.pad_runs && t.maj_drop == 0);
    let t = Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13.geh4");
    assert_eq!((t.rank_hold, t.rank_mode, t.rank_park), (16, 1, 13));
    assert!(t.erase_pairs && t.graft && !t.pad_runs);
    assert_eq!(t.maj_drop, 4);
    assert_eq!(t.hot_groups, 3);
    let t = Tweaks::parse("imchxgrdky3zabCAXJBMnFsUtq618_1.p");
    assert_eq!((t.rank_hold, t.rank_mode, t.rank_park), (618, 1, 0));
    assert!(t.pad_runs);
    let t = Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4");
    assert_eq!((t.rank_hold, t.rank_park), (0, 0));
    assert!(t.erase_pairs && t.graft && t.maj_drop == 4 && t.hot_groups == 4);
}
