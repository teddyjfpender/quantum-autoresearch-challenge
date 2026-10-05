//! Dumps the split one-hot layout and the angle words of a
//! bundle, for offline analysis of the angle-delivery structure. No circuit is built.
//!
//! `SA_SPECS`, `SA_TWEAKS` (one bundle), `SA_MU` as in `tests_combo`; `SA_DUMP` the output path.
use super::tables::SaTables;
use super::{lane_map, onehot};
use crate::spec::sa::SaSpec;
use std::fmt::Write as _;

#[test]
#[ignore = "pinned specs; analysis dump"]
fn dump_layout() {
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-est-v1".into());
    let tw = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky3zabCHKVIXD".into());
    let out = std::env::var("SA_DUMP").expect("SA_DUMP path");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let boxed = crate::spec::load(root, &id).unwrap();
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let p = super::tests_combo::params(s, &tw);
    let map = lane_map(s, p).unwrap();
    let t = SaTables::with_items(s, &map, p.tw.derive_id);
    let g = p.tw.hot_groups as usize;
    let (fold, e1) = onehot::class_layout(&t, g.max(1), p.tw.class_mixed);
    let n = onehot::leaves(&t);
    let mut txt = String::new();
    writeln!(
        txt,
        "N {} R {} B {} E {} e1 {} G {} widths {:?}",
        s.n, s.r, s.b, n, e1, g, t.widths
    )
    .unwrap();
    for gi in 0..g.max(1) {
        for c in 0..e1 {
            if let Some(e) = fold.leaf_at[gi][c] {
                let angs: Vec<String> = (0..t.widths.len())
                    .map(|j| onehot::angle(&t, e, j).to_string())
                    .collect();
                writeln!(txt, "L {gi} {c} {e} {}", angs.join(" ")).unwrap();
            }
        }
    }
    std::fs::write(out, txt).unwrap();
}
