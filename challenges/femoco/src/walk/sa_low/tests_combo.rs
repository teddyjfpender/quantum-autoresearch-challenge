//! Static counts and the peak anatomy of `sa-toff` points on any pinned sos-sa spec,
//! the tapered ones included (spec/SPEC-SA.md section 13). No lanes; release only.
//!
//! - `combo_scan`: the expected-count ledger's `C_step` with every `Givens` charged at its own
//!   width (`2 (w_j - 2)`, `beta` when the spec has no `rotation_widths` block), and the static
//!   `Q_peak` from the harness's own compile pass (`sim::compile_sa`, plus the phase gradient).
//! - `anatomy`: the live ancillas at the first op that reaches `Q_peak`, grouped by the ledger
//!   stage that first touched them (`Ledger::ends`), so a peak cut can be aimed at the registers
//!   that are actually live there.
//!
//! Both read `SA_SPECS` (comma separated), `SA_TWEAKS` (comma separated letter bundles),
//! `SA_MU` (`o,i`; default the spec's own) and `SA_INNER_A` / `SA_OUTER_A` (run-time overrides of
//! the build-time knobs).
use super::{emit, lane_map, Params, Tweaks};
use crate::circuit::{Builder, Op, OperationType as K, NONE, SEG_INNER_BEGIN};
use crate::lanemap::LaneMap;
use crate::sim::givens_tracker;
use crate::spec::sa::SaSpec;
use crate::spec::EncodingSpec;
use std::collections::BTreeMap;

pub(super) fn pinned(id: &str) -> Box<dyn EncodingSpec> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::spec::load(root, id).unwrap()
}

fn env_list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_else(|_| default.into())
        .split(',')
        .map(str::to_string)
        .collect()
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

pub(super) fn params(s: &SaSpec, tw: &str) -> Params {
    let base = Params::for_spec(s);
    let mu: Option<(u32, u32)> = std::env::var("SA_MU").ok().map(|v| {
        let x: Vec<u32> = v.split(',').map(|y| y.parse().unwrap()).collect();
        (x[0], x[1])
    });
    Params {
        tw: Tweaks::parse(tw),
        outer: mu.map_or(base.outer, |m| (base.outer.0, m.0)),
        inner: mu.map_or(base.inner, |m| (base.inner.0, m.1)),
        inner_a: env_usize("SA_INNER_A").unwrap_or(base.inner_a),
        outer_a: env_usize("SA_OUTER_A").unwrap_or(base.outer_a),
        ..base
    }
}

/// Expected `C_step`: the ledger's expected Toffolis (the copy twice), every `Givens` at its
/// width, the two reflections and the four spin swaps.
fn c_step(led: &super::ledger::Ledger, s: &SaSpec, p: Params) -> f64 {
    let map = lane_map(s, p).unwrap();
    let (to, go) = led.expected_sum("outer");
    let (tc, gc) = led.expected_sum("copy");
    let giv = go + 2 * gc;
    let rot = (s.n - 1) as u64;
    assert_eq!(giv % rot, 0, "every network applies all N - 1 rotations");
    let per_net: f64 = match &s.widths {
        Some(w) => w.iter().map(|&x| 2.0 * f64::from(x - 2)).sum(),
        None => rot as f64 * 2.0 * f64::from(s.beta - 2),
    };
    let swaps = if p.swap { 4.0 * (s.n + 1) as f64 } else { 0.0 };
    to + 2.0 * tc
        + (giv / rot) as f64 * per_net
        + f64::from(map.uniform_bits() - 2)
        + f64::from(map.inner_width() - 2)
        + swaps
}

pub(super) struct Built {
    pub(super) ops: Vec<Op>,
    pub(super) led: super::ledger::Ledger,
    pub(super) first: u32,
    pub(super) registers: Vec<Vec<u32>>,
    pub(super) q_peak: u64,
}

pub(super) fn build(s: &SaSpec, p: Params) -> Built {
    let map = lane_map(s, p).unwrap();
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let led = emit(s, &map, &mut b, p);
    let ops = b.finish();
    let layout = crate::sim::Layout {
        system: s.system_qubits(),
        uniform: map.uniform_bits(),
    };
    let inner = map
        .inner_bits()
        .map(|(lo, width)| crate::sim::Inner { lo, width });
    let tr = givens_tracker(s);
    let c = crate::sim::compile_sa(&ops, &layout, tr, inner).unwrap();
    Built {
        first: layout.first_ancilla(),
        registers: c.registers.clone(),
        q_peak: c.q_peak + tr.map_or(0, |t| t.phase_gradient_qubits()),
        ops,
        led,
    }
}

#[test]
#[ignore = "pinned specs; run in release"]
fn combo_scan() {
    for id in env_list("SA_SPECS", "reiher-sa-est-v1") {
        let boxed = pinned(&id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for tw in env_list("SA_TWEAKS", "imchxL,imchxlgr") {
            let p = params(s, &tw);
            let bt = build(s, p);
            let c = c_step(&bt.led, s, p);
            println!(
                "COMBO {id} {tw} mu ({}, {}) inner_a {} outer_a {}: C_step {c:.1} Q_peak {} product {:.4e}",
                p.outer.1,
                p.inner.1,
                p.inner_a,
                p.outer_a,
                bt.q_peak,
                c * bt.q_peak as f64
            );
            // SA_STAGES=1: the expected Toffolis of every ledger row (a copy row counts once
            // here; the copy runs twice).
            if std::env::var("SA_STAGES").is_ok() {
                for (r, e) in bt.led.rows.iter().zip(&bt.led.expected) {
                    println!("  {e:>9.1}  {:>4} Givens  {}", r.2, r.0);
                }
            }
        }
    }
}

/// The live ancillas at the first op where the count reaches its maximum (the harness's
/// liveness rule, `sim::liveness`), grouped by the ledger stage that first touched each one.
#[test]
#[ignore = "pinned specs; run in release"]
fn anatomy() {
    for id in env_list("SA_SPECS", "li-sa-est-v1") {
        let boxed = pinned(&id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for tw in env_list("SA_TWEAKS", "imchxL") {
            let p = params(s, &tw);
            let bt = build(s, p);
            let mut birth: Vec<Option<usize>> = Vec::new();
            let (mut count, mut best, mut at) = (0usize, 0usize, 0usize);
            let mut snap: Vec<usize> = Vec::new();
            let mut depth = 0usize;
            // SA_STAGEMAX=1: the live-ancilla maximum of every ledger stage.
            let mut per_op: Vec<usize> = Vec::with_capacity(bt.ops.len());
            let touch = |q: u32, i: usize, birth: &mut Vec<Option<usize>>, count: &mut usize| {
                if q < bt.first {
                    return;
                }
                let k = (q - bt.first) as usize;
                if k >= birth.len() {
                    birth.resize(k + 1, None);
                }
                if birth[k].is_none() {
                    birth[k] = Some(i);
                    *count += 1;
                }
            };
            for (i, op) in bt.ops.iter().enumerate() {
                match op.kind {
                    K::PushCondition => depth += 1,
                    K::PopCondition => depth -= 1,
                    _ => {}
                }
                match op.kind {
                    K::Segment | K::Register | K::AppendToRegister | K::DebugPrint => {}
                    K::R | K::Hmr if depth == 0 && op.c_condition == NONE => {
                        if let Some(k) = op.q_target.checked_sub(bt.first) {
                            if let Some(slot) = birth.get_mut(k as usize) {
                                if slot.take().is_some() {
                                    count -= 1;
                                }
                            }
                        }
                    }
                    K::Givens => {
                        for &q in bt.registers.get(op.r_target as usize).into_iter().flatten() {
                            touch(q, i, &mut birth, &mut count);
                        }
                    }
                    _ => op
                        .qubits()
                        .for_each(|q| touch(q, i, &mut birth, &mut count)),
                }
                // SA_PEAKSTAGE=<text>: the peak among the ops of the ledger stages
                // whose name contains the text (first copy), not the global one.
                let in_stage = std::env::var("SA_PEAKSTAGE").map_or(true, |st| {
                    let r = bt.led.ends.partition_point(|&e| e <= i);
                    bt.led.rows.get(r).is_some_and(|row| row.0.contains(&st))
                });
                if in_stage && count > best {
                    best = count;
                    at = i;
                    snap = birth.iter().filter_map(|b| *b).collect();
                }
                per_op.push(count);
            }
            // The inner copy is emitted once and repeated byte for byte after the reflection
            // (`Builder::nested_inner`); the ledger saw the first copy only, so an op in the second
            // copy is attributed to its twin in the first.
            let begins: Vec<usize> = bt
                .ops
                .iter()
                .enumerate()
                .filter(|(_, o)| o.kind == K::Segment && o.r_target == SEG_INNER_BEGIN)
                .map(|(i, _)| i + 1)
                .collect();
            let fold = |i: usize| -> (usize, bool) {
                match begins.as_slice() {
                    [a, b] if i >= *b && i - b < b - a => (a + (i - b), true),
                    _ => (i, false),
                }
            };
            let row_of = |i: usize| -> (usize, bool) {
                let (j, second) = fold(i);
                (bt.led.ends.partition_point(|&e| e <= j), second)
            };
            let name = |(r, second): (usize, bool)| -> String {
                let base = bt
                    .led
                    .rows
                    .get(r)
                    .map_or_else(|| "(after the ledger)".into(), |row| row.0.clone());
                if second {
                    format!("{base} [second copy]")
                } else {
                    base
                }
            };
            let mut groups: BTreeMap<(usize, bool), usize> = BTreeMap::new();
            for &b in &snap {
                *groups.entry(row_of(b)).or_insert(0) += 1;
            }
            println!(
                "ANATOMY {id} {tw} mu ({}, {}): Q_peak {} = base {} + ancillas {best} + phase gradient; peak at op {at} in stage '{}'",
                p.outer.1,
                p.inner.1,
                bt.q_peak,
                bt.first,
                name(row_of(at))
            );
            for (key, n) in groups {
                println!("  {n:>5}  {}", name(key));
            }
            // SA_PEAKOPS=1: the ops around the peak and, for each qubit live
            // there, the op that first touched it (to see what a stage holds at its peak).
            if std::env::var("SA_PEAKOPS").is_ok() {
                for j in at.saturating_sub(6)..=at {
                    let o = &bt.ops[j];
                    println!(
                        "  op {j}: {:?} t {} c1 {} c2 {}",
                        o.kind, o.q_target, o.q_control1, o.q_control2
                    );
                }
                let mut bs: Vec<usize> = snap.clone();
                bs.sort_unstable();
                for b in bs
                    .iter()
                    .rev()
                    .filter(|&&b| std::env::var("SA_PEAKOPS").unwrap_or_default() == "all" || b > 0)
                {
                    let o = &bt.ops[*b];
                    println!(
                        "  born at op {b} ({}): {:?} t {}",
                        name(row_of(*b)),
                        o.kind,
                        o.q_target
                    );
                }
            }
            // SA_GIVENSPROF=1: the live count (as Q) at every first-copy Givens
            // of SELECT and between them (the maximum since the previous Givens), to see where a
            // per-position delivery budget has room.
            if std::env::var("SA_GIVENSPROF").is_ok() {
                let off = bt.q_peak as usize - best;
                let mut last = 0usize;
                let mut line = String::new();
                let mut seen = 0usize;
                for (i, op) in bt.ops.iter().enumerate() {
                    if op.kind == K::Givens && !fold(i).1 {
                        let gap = per_op[last..i].iter().max().copied().unwrap_or(0);
                        line.push_str(&format!(" {}/{}", off + per_op[i], off + gap));
                        last = i + 1;
                        seen += 1;
                        if seen.is_multiple_of(16) {
                            line.push('\n');
                        }
                    }
                }
                println!("GIVENSPROF (at Givens / max since previous):{line}");
            }
            if let Ok(r) = std::env::var("SA_DUMP") {
                let (a, z) = r.split_once("..").unwrap();
                let (a, z): (usize, usize) = (a.parse().unwrap(), z.parse().unwrap());
                for j in a..z {
                    let o = &bt.ops[j];
                    println!(
                        "  dump {j} ({}): {:?} t {} c1 {} c2 {} cc {}",
                        name(row_of(j)),
                        o.kind,
                        o.q_target,
                        o.q_control1,
                        o.q_control2,
                        o.c_condition
                    );
                }
            }
            if std::env::var("SA_STAGEMAX").is_ok() {
                let mut maxes: BTreeMap<(usize, bool), usize> = BTreeMap::new();
                for (i, &c) in per_op.iter().enumerate() {
                    let e = maxes.entry(row_of(i)).or_insert(0);
                    *e = (*e).max(c);
                }
                for (key, n) in maxes {
                    println!(
                        "  max {:>5}  {}",
                        bt.first as usize + n + (bt.q_peak as usize - bt.first as usize - best),
                        name(key)
                    );
                }
            }
        }
    }
}

/// GF(2) rank of the RPREP angle data: row `e` of the matrix is leaf `e`'s angle word
/// (every rotation's register value, [`super::onehot::angle`]). Any register `x(v)` from which
/// every angle bit is a CNOT fan-out (a linear function of `x`) needs at least the affine rank of
/// these rows; the one-hot (`E` qubits) and the angle word (`sum w_j` qubits) are two such
/// registers. Prints `E`, `sum w_j`, the rank and the affine rank.
#[test]
#[ignore = "pinned specs; run in release"]
fn onehot_rank() {
    for id in env_list("SA_SPECS", "reiher-sa-v1,li-sa-v1,li-sa-gb-r14-v1") {
        let boxed = pinned(&id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = params(s, "imchxgr");
        let map = lane_map(s, p).unwrap();
        let t = super::tables::SaTables::with(s, &map, p.tw.derive_id, p.tw.compact_outer);
        let e = super::onehot::leaves(&t);
        let cols: usize = t.widths.iter().sum();
        let row = |i: usize| -> Vec<u64> {
            let mut w = vec![0u64; cols.div_ceil(64)];
            let mut at = 0;
            for (j, &wj) in t.widths.iter().enumerate() {
                let a = super::onehot::angle(&t, i, j);
                for k in 0..wj {
                    if a >> k & 1 == 1 {
                        w[(at + k) / 64] |= 1 << ((at + k) % 64);
                    }
                }
                at += wj;
            }
            w
        };
        let rank = |rows: Vec<Vec<u64>>| -> usize {
            let mut basis: Vec<Vec<u64>> = Vec::new();
            let mut piv: Vec<usize> = Vec::new();
            for mut r in rows {
                for (bv, &pc) in basis.iter().zip(&piv) {
                    if r[pc / 64] >> (pc % 64) & 1 == 1 {
                        for (x, y) in r.iter_mut().zip(bv) {
                            *x ^= y;
                        }
                    }
                }
                if let Some(pc) = (0..cols).find(|&c| r[c / 64] >> (c % 64) & 1 == 1) {
                    for (bv, _) in basis.iter_mut().zip(&piv) {
                        if bv[pc / 64] >> (pc % 64) & 1 == 1 {
                            for (x, y) in bv.iter_mut().zip(&r) {
                                *x ^= y;
                            }
                        }
                    }
                    basis.push(r);
                    piv.push(pc);
                }
            }
            basis.len()
        };
        let rows: Vec<Vec<u64>> = (0..e).map(row).collect();
        let r0 = rows[0].clone();
        let aff: Vec<Vec<u64>> = rows
            .iter()
            .map(|r| r.iter().zip(&r0).map(|(x, y)| x ^ y).collect())
            .collect();
        let widths_hist: BTreeMap<usize, usize> =
            t.widths.iter().fold(BTreeMap::new(), |mut m, &w| {
                *m.entry(w).or_insert(0) += 1;
                m
            });
        println!(
            "RANK {id}: N {} R {} B {} E {e} (squares {} + one-body {}), angle word {cols} bits, widths {widths_hist:?}, rank {}, affine rank {}",
            s.n,
            s.r,
            s.b,
            s.r * s.b,
            s.n,
            rank(rows),
            rank(aff)
        );
    }
}

/// How many items of each square's inner table hold more than one bucket's
/// worth of lanes (`count > 2^mu`). A Walker table can send every small bucket's alt to one of
/// those heavy items, so this is the number of distinct alt values a table needs.
#[test]
#[ignore = "pinned specs; run in release"]
fn alias_heavy() {
    for id in env_list("SA_SPECS", "reiher-sa-est-v1,li-sa-est-v1") {
        let boxed = pinned(&id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = params(s, "imchxgr");
        let map = lane_map(s, p).unwrap();
        let cap = 1u64 << p.inner.1;
        let mut hist: BTreeMap<usize, usize> = BTreeMap::new();
        let mut sizes: BTreeMap<usize, usize> = BTreeMap::new();
        for t in map.inner.iter().skip(s.n) {
            let c = t.counts(s.b + 1);
            let heavy = c.iter().filter(|&&x| x > cap).count();
            *hist.entry(heavy).or_insert(0) += 1;
            let distinct: std::collections::BTreeSet<u32> = t
                .alt
                .iter()
                .zip(&t.keep)
                .enumerate()
                .filter(|(i, (_, &k))| u64::from(k) < cap && *i < (1 << t.k))
                .map(|(_, (&a, _))| a)
                .collect();
            *sizes.entry(distinct.len()).or_insert(0) += 1;
        }
        println!("HEAVY {id}: tables by number of heavy items {hist:?}");
        println!("HEAVY {id}: tables by distinct alt values (current layout) {sizes:?}");
    }
}

/// How many alias buckets are self-aliased (`alt[i] = i`, every lane of the
/// bucket goes to its own item) in the outer table and in the square inner tables, per spec.
/// Lever `W` (the outer keep test erased through the item-index witness) needs these to be
/// handled: on a self bucket the witness `[item = i]` is 1 whatever the keep test says.
#[test]
#[ignore = "pinned specs; run in release"]
fn self_buckets() {
    for id in env_list("SA_SPECS", "reiher-sa-est-v1,li-sa-est-v1") {
        let spec = pinned(&id);
        let s = spec.as_any().downcast_ref::<SaSpec>().unwrap();
        let p = params(s, "imchxgrdky3zfab");
        let map = lane_map(s, p).unwrap();
        let o = &map.outer;
        let items = s.outer_items();
        let self_o = (0..items).filter(|&i| o.alt[i] as usize == i).count();
        let full_o = (0..items)
            .filter(|&i| o.alt[i] as usize == i && o.keep[i] > 0)
            .count();
        let (mut self_i, mut tot_i) = (0, 0);
        for t in map.inner.iter().skip(s.n) {
            for i in 0..t.keep.len().min(s.b + 1) {
                tot_i += 1;
                if t.alt[i] as usize == i {
                    self_i += 1;
                }
            }
        }
        println!(
            "SELF {id}: outer {self_o} of {items} items self-aliased ({full_o} with keep > 0); inner {self_i} of {tot_i}"
        );
    }
}

/// Per ledger stage (first copy), the non-system qubits live at the stage's own
/// peak that no op of the stage touches (a `Givens` touches its angle register). These are the
/// candidates for a borrowed-workspace (dirty-ancilla) construction inside that stage. System
/// qubits are excluded: the harness lets a system qubit only be a Pauli target (`sim::compile`,
/// `lower_system`), so no construction can read one as a control. Prints, per stage: the stage
/// peak, the idle uniform bits (of `u`), the idle ancillas, and the idle ancillas by birth stage.
#[test]
#[ignore = "pinned specs; run in release"]
fn idle_census() {
    for id in env_list("SA_SPECS", "reiher-sa-est-v1") {
        let boxed = pinned(&id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for tw in env_list("SA_TWEAKS", "imchxgrdky3zabCHKVIDXtNBWs+") {
            let p = params(s, &tw);
            let bt = build(s, p);
            let nsys = s.system_qubits() as u32;
            let ubase = 1 + nsys;
            let nrows = bt.led.rows.len();
            let begins: Vec<usize> = bt
                .ops
                .iter()
                .enumerate()
                .filter(|(_, o)| o.kind == K::Segment && o.r_target == SEG_INNER_BEGIN)
                .map(|(i, _)| i + 1)
                .collect();
            // Fold: the second copy repeats the first byte for byte; its ops are skipped for the
            // per-stage census and later ops are shifted back onto the ledger's indices.
            let fold = |i: usize| -> Option<usize> {
                match begins.as_slice() {
                    [a, b] if i >= *b && i - b < b - a => None,
                    [a, b] if i >= *b => Some(i - (b - a)),
                    _ => Some(i),
                }
            };
            let stage_of = |i: usize| bt.led.ends.partition_point(|&e| e <= i);
            let mut touched: Vec<std::collections::BTreeSet<u32>> =
                vec![Default::default(); nrows + 1];
            let mut birth: Vec<Option<usize>> = Vec::new();
            let mut count = 0usize;
            let mut depth = 0usize;
            let mut best: Vec<(usize, Vec<u32>)> = vec![(0, Vec::new()); nrows + 1];
            let mut gmax = 0usize;
            let mut born: Vec<usize> = Vec::new();
            let mut born_at: Vec<Vec<usize>> = vec![Vec::new(); nrows + 1];
            for (i, op) in bt.ops.iter().enumerate() {
                let fi = fold(i);
                let r = fi.map_or(nrows, |j| stage_of(j).min(nrows));
                match op.kind {
                    K::PushCondition => depth += 1,
                    K::PopCondition => depth -= 1,
                    _ => {}
                }
                let mut names: Vec<u32> = Vec::new();
                match op.kind {
                    K::Segment | K::Register | K::AppendToRegister | K::DebugPrint => {}
                    K::R | K::Hmr if depth == 0 && op.c_condition == NONE => {
                        if let Some(k) = op.q_target.checked_sub(bt.first) {
                            if let Some(slot) = birth.get_mut(k as usize) {
                                if slot.take().is_some() {
                                    count -= 1;
                                }
                            }
                        }
                        names.push(op.q_target);
                    }
                    K::Givens => {
                        names.extend(bt.registers.get(op.r_target as usize).into_iter().flatten())
                    }
                    _ => names.extend(op.qubits()),
                }
                let frees =
                    matches!(op.kind, K::R | K::Hmr) && depth == 0 && op.c_condition == NONE;
                for &q in &names {
                    touched[r].insert(q);
                    if frees || q < bt.first {
                        continue;
                    }
                    let k = (q - bt.first) as usize;
                    if k >= birth.len() {
                        birth.resize(k + 1, None);
                        born.resize(k + 1, nrows);
                    }
                    if birth[k].is_none() {
                        birth[k] = Some(i);
                        born[k] = r;
                        count += 1;
                    }
                }
                gmax = gmax.max(count);
                if r < nrows && (count > best[r].0 || best[r].1.is_empty() && count > 0) {
                    born_at[r] = born.clone();
                    best[r] = (
                        count,
                        birth
                            .iter()
                            .enumerate()
                            .filter(|(_, b)| b.is_some())
                            .map(|(k, _)| bt.first + k as u32)
                            .collect(),
                    );
                }
            }
            let off = bt.q_peak as usize - gmax; // control + system + uniform + gradient
            println!(
                "IDLE {id} {tw}: Q_peak {} (base {} + gradient {})",
                bt.q_peak,
                bt.first,
                off - bt.first as usize
            );
            for r in 0..nrows {
                let (c, live) = &best[r];
                let idle_u = (ubase..bt.first)
                    .filter(|q| !touched[r].contains(q))
                    .count();
                let idle: Vec<u32> = live
                    .iter()
                    .copied()
                    .filter(|q| !touched[r].contains(q))
                    .collect();
                let mut by: BTreeMap<usize, usize> = BTreeMap::new();
                for &q in &idle {
                    let k = (q - bt.first) as usize;
                    *by.entry(born_at[r].get(k).copied().unwrap_or(nrows))
                        .or_insert(0) += 1;
                }
                let names: Vec<String> = by
                    .iter()
                    .map(|(k, n)| {
                        format!(
                            "{n} from '{}'",
                            bt.led.rows.get(*k).map_or("?", |x| x.0.as_str())
                        )
                    })
                    .collect();
                println!(
                    "  stage peak {:>4} idle uniform {:>2} / {} idle ancillas {:>3}  {}  [{}]",
                    off + c,
                    idle_u,
                    bt.first - ubase,
                    idle.len(),
                    bt.led.rows[r].0,
                    names.join("; ")
                );
            }
        }
    }
}
