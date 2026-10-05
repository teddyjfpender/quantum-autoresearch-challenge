//! Data probes for the non-rotation work of the SA step.
//!
//! - `dump_tables`: the outer and inner alias tables and the item fields of a pinned spec at a
//!   keep split, as text (`SA_DUMP=<path>`), for offline analysis.
use super::lane_map;
use super::tests_combo::{params, pinned};
use crate::spec::sa::SaSpec;
use std::fmt::Write as _;

#[test]
#[ignore = "pinned specs; run in release"]
fn dump_tables() {
    let id = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1".into());
    let boxed = pinned(&id);
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let p = params(s, "imchxgrdky");
    let map = lane_map(s, p).unwrap();
    let mut out = String::new();
    writeln!(
        out,
        "spec {id} n {} r {} b {} c {} beta {}",
        s.n, s.r, s.b, s.c, s.beta
    )
    .unwrap();
    let o = &map.outer;
    writeln!(out, "outer k {} mu {}", o.k, o.mu).unwrap();
    for (i, (k, a)) in o.keep.iter().zip(&o.alt).enumerate() {
        writeln!(out, "o {i} {k} {a}").unwrap();
    }
    for (q, t) in map.inner.iter().enumerate() {
        writeln!(out, "inner {q} k {} mu {}", t.k, t.mu).unwrap();
        for (i, (k, a)) in t.keep.iter().zip(&t.alt).enumerate() {
            writeln!(out, "i {q} {i} {k} {a}").unwrap();
        }
    }
    for (r, e) in s.e.iter().enumerate() {
        writeln!(out, "e {r} {e}").unwrap();
    }
    for (q, wb) in s.wb.iter().enumerate() {
        writeln!(out, "wb {q} {wb}").unwrap();
    }
    for (x, w) in s.w.iter().enumerate() {
        writeln!(out, "w {x} {w}").unwrap();
    }
    std::fs::write(std::env::var("SA_DUMP").unwrap(), out).unwrap();
}
