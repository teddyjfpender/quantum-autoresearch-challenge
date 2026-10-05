//! The Low et al. walk end to end on small sos-sa specs with dyadic weights (exact lane maps):
//! it passes `score::evaluate` with the shipped taxonomy and verified axes, the ledger's counts
//! add up to the harness's `C_step`, and mutated circuits are rejected. The pinned-spec checks
//! (`pinned_*`) are `#[ignore]`d; run them in release.
use super::{emit, family, family_toff, lane_map, Params, Tweaks};
use crate::circuit::{read_ops, write_ops, Builder, Op, OperationType as K, OpsFile};
use crate::equiv::evaluate_checked as evaluate;
use crate::lanemap::LaneMap;
use crate::score::{Evaluation, FamilyOut, Inputs};
use crate::sim::givens_tracker;
use crate::spec::sa::{parse_payload, SaSpec};
use crate::spec::{EncodingSpec, Exact};
use crate::taxonomy::{self, AxisStatus};
use std::sync::atomic::{AtomicU64, Ordering};

const BETA: u32 = 8;

fn angles(seed: u64, count: usize) -> Vec<u32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..count)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (1 << BETA)) as u32
        })
        .collect()
}

/// `FEMOSAS1` for `n` orbitals and `(R, B, C)`.
fn payload(n: usize, rbc: (usize, usize, usize), e: &[f64], wb: &[f64], w: &[f64]) -> Vec<u8> {
    let (r, bb, c) = rbc;
    let mut out = b"FEMOSAS1".to_vec();
    for v in [1u32, n as u32, r as u32, bb as u32, c as u32, BETA, 3] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(-2.5f64).to_le_bytes());
    e.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    angles(30, n * (n - 1))
        .iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    wb.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    w.iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    angles(31, r * bb * (n - 1))
        .iter()
        .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    out
}

/// N = 3, (R, B, C) = (1, 2, 2): one-body weights 2|e| = 6, 2, 8; squares S^2/2 = 8, 8 (outer
/// total 32); inner (1, 1, 2) and (2, 1, 1). Every weight is dyadic.
fn spec3() -> SaSpec {
    let bytes = payload(
        3,
        (1, 2, 2),
        &[3.0, -1.0, 4.0],
        &[2.0, -1.0],
        &[1.0, 1.0, 2.0, -1.0],
    );
    parse_payload("test-sa-v1", &bytes).unwrap()
}

/// N = 4, (R, B, C) = (2, 3, 1): 6 outer items in 8 buckets (padding), 4 inner items in 4
/// buckets, one-body indices past `2^k_i` (so `hi` takes two values for one-body lanes). The
/// outer weights are not dyadic fractions of their total, so the lane map is not exact.
fn spec4() -> SaSpec {
    let bytes = payload(
        4,
        (2, 3, 1),
        &[1.0, -2.0, 1.0, 4.0],
        &[1.0, -4.0],
        &[1.0, -1.0, 1.0, 2.0, -1.0, 1.0],
    );
    parse_payload("test-sa4-v1", &bytes).unwrap()
}

/// N = 5, (R, B, C) = (1, 2, 2): one-body weights 6, 2, 4, 2, 2 and the squares of `spec3`
/// (outer total 32, exact), with four rotations per network, so the angles can be streamed in
/// two chunks of two or four of one.
fn spec5() -> SaSpec {
    let bytes = payload(
        5,
        (1, 2, 2),
        &[3.0, -1.0, 2.0, 1.0, -1.0],
        &[2.0, -1.0],
        &[1.0, 1.0, 2.0, -1.0],
    );
    parse_payload("test-sa5-v1", &bytes).unwrap()
}

const P3: Params = Params {
    outer: (3, 2),
    inner: (2, 1),
    outer_a: 1,
    inner_a: 1,
    swap: false,
    chunks: 1,
    outer_erase: false,
    dense: false,
    tw: Tweaks::OFF,
    pareto: false,
    lean: false,
    carries: 0,
    drop_alt: false,
    narrow: super::pareto::Narrow::OFF,
};

/// Every `sa-toff` lever alone, the default bundle and the bundle with held ladders.
const TWEAKS: [&str; 10] = [
    "i", "m", "mc", "x", "h", "l", "L", "all", "imchxl", "imchxL",
];

static TMP: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Declare the `sa-toff` family in [`eval`] (set by the `sa-toff` tests).
    static TOFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-sa-low-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

fn eval(s: &SaSpec, lanemap: &[u8], ops: &[Op], samples: usize) -> Result<Evaluation, String> {
    let swap = ops
        .iter()
        .any(|o| matches!(o.kind, K::SpinSwap | K::SpinSwapDg));
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    let tax = taxonomy::load_taxonomy(&path).unwrap();
    let check = |f: &_, facts: &_| taxonomy::check(&tax, f, facts);
    let toff = TOFF.with(std::cell::Cell::get);
    let fam = serde_json::to_vec(&FamilyOut {
        family: if toff { family_toff() } else { family(swap) },
        spec: s.id().to_string(),
    })
    .unwrap();
    let file = ops_file(ops);
    evaluate(&Inputs {
        spec: s,
        lanemap,
        family: &fam,
        ops: &file,
        samples,
        tracker: givens_tracker(s),
        check: &check,
    })
}

fn build(s: &SaSpec, p: Params) -> (Vec<u8>, Vec<Op>, super::ledger::Ledger) {
    let map = lane_map(s, p).unwrap();
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let led = emit(s, &map, &mut b, p);
    (map.to_bytes(), b.finish(), led)
}

/// Static `C_step` from the ledger: Toffolis (the copy twice), Givens at `2 (beta - 2)`, and
/// the two reflections.
fn ledger_c_step(led: &super::ledger::Ledger, beta: u32, u: u32, w: u32) -> f64 {
    let (to, go) = led.sum("outer");
    let (tc, gc) = led.sum("copy");
    let tof = to + 2 * tc;
    let giv = go + 2 * gc;
    (tof + giv * 2 * u64::from(beta - 2) + u64::from(u - 2) + u64::from(w - 2)) as f64
}

#[test]
fn walk_passes_on_an_exact_spec() {
    for swap in [false, true] {
        passes_on_an_exact_spec(Params { swap, ..P3 });
    }
}

/// The streamed angles (`sa-lowq`) on an exact five-orbital spec: two chunks of two rotations
/// and four of one, with and without the spin swap. `passes_on` also checks that the ledger's
/// transitions add up to the harness's count.
#[test]
fn streamed_walk_passes_on_an_exact_spec() {
    let s = spec5();
    for chunks in [1, 2, 3, 4] {
        for swap in [false, true] {
            for (outer_erase, dense) in [(false, false), (true, false), (false, true), (true, true)]
            {
                passes_on(
                    &s,
                    Params {
                        swap,
                        chunks,
                        outer_erase,
                        dense,
                        ..P3
                    },
                );
            }
        }
    }
}

/// A streamed circuit that rotates by the wrong slot is rejected, and so is an outer-erase circuit
/// without its phase fixes; the build is deterministic and two chunks emit a different stream
/// from one.
#[test]
fn streamed_mutants_are_rejected() {
    let s = spec5();
    for swap in [false, true] {
        let p = Params {
            swap,
            chunks: 2,
            ..P3
        };
        let (lm, ops, _) = build(&s, p);
        // The first Givens reading slot register r now reads slot r + 1 (the slots' registers
        // are declared consecutively).
        let m1 = mutate_all(&ops, (|o: &Op| o.kind == K::Givens, 4), 0, |mut o| {
            o.r_target += 1;
            Some(o)
        });
        let e = eval(&s, &lm, &m1, 1 << 12);
        assert!(
            e.is_err(),
            "wrong-slot mutant must be rejected (swap {swap})"
        );
        println!("slot: {}", e.err().unwrap());
        // With the outer garbage measured early: its phase fixes dropped (the conditioned
        // CZs, then the conditioned Zs).
        let pe = Params {
            outer_erase: true,
            ..p
        };
        let (lm_e, ops_e, _) = build(&s, pe);
        let nz = crate::circuit::NONE;
        for kind in [K::CZ, K::Z] {
            let m: Vec<Op> = ops_e
                .iter()
                .filter(|o| !(o.kind == kind && o.c_condition != nz))
                .copied()
                .collect();
            assert!(m.len() < ops_e.len());
            let e = eval(&s, &lm_e, &m, 1 << 12);
            assert!(e.is_err(), "{kind:?} phase-fix mutant must be rejected");
            println!("outer erase {kind:?}: {}", e.err().unwrap());
        }
        let map = lane_map(&s, p).unwrap();
        let mut b = crate::circuit::Builder::new(2 * s.n);
        b.declare_uniform(map.uniform_bits());
        let _ = emit(&s, &map, &mut b, p);
        let good = b.finish();
        assert_eq!(good, ops);
        let zero = Params { chunks: 1, ..p };
        let (_, one, _) = build(&s, zero);
        assert_ne!(one, ops, "C = 2 must differ from C = 1");
    }
}

fn passes_on_an_exact_spec(p: Params) {
    passes_on(&spec3(), p);
}

fn passes_on(s: &SaSpec, p: Params) {
    let (lm, ops, led) = build(s, p);
    let ev = eval(s, &lm, &ops, 1 << 14).unwrap();
    assert_eq!(ev.rounding_error, Exact::zero());
    let n = ev.nested.expect("nested stats");
    assert!(ev.facts.nested_validated);
    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
        assert_eq!(v.status, AxisStatus::Verified, "{axis}");
    }
    let map = lane_map(s, p).unwrap();
    let want =
        ledger_c_step(&led, BETA, map.uniform_bits(), map.inner_width()) + ev.spin_swap_toffoli;
    if p.swap {
        // Two SpinSwaps per copy, N + 1 Toffolis each, on every lane.
        assert!((ev.spin_swaps - 4.0).abs() < 1e-12, "{}", ev.spin_swaps);
        assert!((ev.spin_swap_toffoli - 4.0 * (s.n + 1) as f64).abs() < 1e-12);
    } else {
        assert_eq!(ev.spin_swaps, 0.0);
    }
    assert!(
        (ev.toffoli - want).abs() < 1e-9,
        "harness {} vs ledger {want}",
        ev.toffoli
    );
    println!(
        "N {} swap {} chunks {} C_step {} Q_peak {} paired {} diagonal {}",
        s.n, p.swap, p.chunks, ev.toffoli, ev.qubits, n.paired_samples, n.diagonal_samples
    );
    for r in &led.rows {
        println!("{:>8} {:>5}  {}", r.1, r.2, r.0);
    }
}

#[test]
fn toff_walk_passes_on_an_exact_spec() {
    TOFF.with(|c| c.set(true));
    for tw in TWEAKS {
        for swap in [true, false] {
            println!("== tweaks {tw} swap {swap}");
            passes_on_an_exact_spec(Params {
                swap,
                tw: Tweaks::parse(tw),
                ..P3
            });
        }
    }
}

#[test]
fn toff_every_block_count_passes() {
    TOFF.with(|c| c.set(true));
    let s = spec3();
    for (oa, ia, swap) in [(0, 0, true), (2, 2, false), (3, 1, true), (0, 2, false)] {
        let p = Params {
            outer_a: oa,
            inner_a: ia,
            swap,
            tw: Tweaks::parse("all"),
            ..P3
        };
        let (lm, ops, _) = build(&s, p);
        eval(&s, &lm, &ops, 1 << 12).unwrap_or_else(|e| panic!("{oa} {ia}: {e}"));
    }
}

#[test]
fn toff_mutants_are_rejected() {
    TOFF.with(|c| c.set(true));
    for swap in [false, true] {
        mutants_rejected(Params {
            swap,
            tw: Tweaks::parse("all"),
            ..P3
        });
    }
}

#[test]
fn toff_walk_runs_on_a_padded_spec() {
    // N = 4 has one-body indices past 2^k_i and padding buckets on both levels: the structural
    // run must fail only on the rounding rule.
    TOFF.with(|c| c.set(true));
    let s = spec4();
    for tw in TWEAKS {
        for swap in [false, true] {
            let p = Params {
                outer: (3, 20),
                inner: (2, 20),
                outer_a: 2,
                inner_a: 1,
                swap,
                tw: Tweaks::parse(tw),
                ..P3
            };
            let (lm, ops, _) = build(&s, p);
            match eval(&s, &lm, &ops, 1 << 13) {
                Ok(ev) => println!(
                    "N=4 {tw} swap {swap} passes: C {} Q {}",
                    ev.toffoli, ev.qubits
                ),
                Err(e) => assert!(e.contains("rounding error"), "{tw} {swap}: {e}"),
            }
        }
    }
}

#[test]
fn every_block_count_passes() {
    let s = spec3();
    for (oa, ia, swap) in [(0, 0, true), (2, 2, false), (3, 1, true), (0, 2, false)] {
        let p = Params {
            outer_a: oa,
            inner_a: ia,
            swap,
            ..P3
        };
        let (lm, ops, _) = build(&s, p);
        eval(&s, &lm, &ops, 1 << 12).unwrap_or_else(|e| panic!("{oa} {ia}: {e}"));
    }
}

/// Replaces every op equal to the `nth` op matching `pick` that occurs an even number of times,
/// at least `times` (an op of
/// the inner copy occurs once per copy, a Givens twice per copy, so both copies stay identical)
/// with `f(op)`, or drops it when `f` returns `None`.
fn mutate_all(
    ops: &[Op],
    (pick, times): (impl Fn(&Op) -> bool, usize),
    nth: usize,
    f: impl Fn(Op) -> Option<Op>,
) -> Vec<Op> {
    let target = *ops
        .iter()
        .filter(|o| {
            let k = ops.iter().filter(|p| *p == *o).count();
            pick(o) && k >= times && k.is_multiple_of(2)
        })
        .nth(nth)
        .expect("op to mutate");
    ops.iter()
        .filter_map(|o| if *o == target { f(*o) } else { Some(*o) })
        .collect()
}

#[test]
fn mutants_are_rejected() {
    for swap in [false, true] {
        mutants_rejected(Params { swap, ..P3 });
    }
}

fn mutants_rejected(p: Params) {
    let s = spec3();
    let (lm, ops, _) = build(&s, p);
    // System qubit j is qubit 1 + j.
    let on_sys = |o: &Op, j: u32| o.q_target == 1 + j;
    // The first controlled Z onto system qubit 0 inside the copies: a pivot.
    let m1 = mutate_all(
        &ops,
        (|o: &Op| o.kind == K::CZ && on_sys(o, 0), 2),
        0,
        |_| None,
    );
    // Without SpinSwap: the first controlled Z onto system qubit 1, another pivot. With it:
    // F instead of F^dagger before V^dagger.
    let m2 = if p.swap {
        mutate_all(&ops, (|o: &Op| o.kind == K::SpinSwapDg, 2), 0, |mut o| {
            o.kind = K::SpinSwap;
            Some(o)
        })
    } else {
        mutate_all(
            &ops,
            (|o: &Op| o.kind == K::CZ && on_sys(o, 1), 2),
            0,
            |_| None,
        )
    };
    // The first controlled X onto system qubit 0: the one-body Majorana.
    let m3 = mutate_all(
        &ops,
        (|o: &Op| o.kind == K::CX && on_sys(o, 0), 2),
        0,
        |_| None,
    );
    // One Givens moved to the other spin's modes (2p + s -> 2p + 1 - s keeps p < q).
    let m4 = mutate_all(
        &ops,
        (
            |o: &Op| o.kind == K::Givens && (o.q_control1 - 1).is_multiple_of(2),
            4,
        ),
        1,
        |mut o| {
            o.q_control1 += 1;
            o.q_target += 1;
            Some(o)
        },
    );
    for (name, m) in [
        ("pivot0", m1),
        ("pivot1", m2),
        ("majorana", m3),
        ("givens", m4),
    ] {
        let e = eval(&s, &lm, &m, 1 << 12);
        assert!(e.is_err(), "{name} mutant must be rejected");
        println!("{name}: {}", e.err().unwrap());
    }
    // (Relabelling a one-body table's items, x -> 1 - x, is not a mutant: M_1^dagger M_0 and
    // M_0^dagger M_1 of the relabelled lane are the same operator, so it must pass.)
    let mut map = lane_map(&s, p).unwrap();
    for t in map.inner.iter_mut().take(s.n) {
        t.alt.iter_mut().for_each(|a| *a ^= 1);
    }
    eval(&s, &map.to_bytes(), &ops, 1 << 12).expect("relabelled one-body items are equivalent");
}

#[test]
fn walk_runs_on_a_padded_spec() {
    // Structural run on N = 4: the lane map is not exact (the rounding error exceeds the rule
    // at these small widths), so evaluate reports that, and nothing else.
    let s = spec4();
    let p = Params {
        outer: (3, 20),
        inner: (2, 20),
        outer_a: 2,
        inner_a: 1,
        swap: false,
        chunks: 1,
        outer_erase: false,
        dense: false,
        tw: Tweaks::default(),
        pareto: false,
        lean: false,
        carries: 0,
        drop_alt: false,
        narrow: super::pareto::Narrow::OFF,
    };
    for swap in [false, true] {
        let (lm, ops, _) = build(&s, Params { swap, ..p });
        match eval(&s, &lm, &ops, 1 << 13) {
            Ok(ev) => println!("N=4 swap {swap} passes: C {} Q {}", ev.toffoli, ev.qubits),
            Err(e) => assert!(e.contains("rounding error"), "{e}"),
        }
    }
}

fn pinned(id: &str) -> Box<dyn EncodingSpec> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::spec::load(root, id).unwrap()
}

/// The per-stage ledger on the pinned specs (no harness run; about 10 s in release).
#[test]
#[ignore = "pinned specs; run in release"]
fn pinned_ledgers() {
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for swap in [false, true] {
            pinned_ledger(
                id,
                s,
                Params {
                    swap,
                    ..Params::for_spec(s)
                },
            );
        }
    }
}

fn pinned_ledger(id: &str, s: &SaSpec, p: Params) {
    {
        let map = lane_map(s, p).unwrap();
        let err = map.rounding_error(s).unwrap().to_f64();
        let mut b = Builder::new(2 * s.n);
        b.declare_uniform(map.uniform_bits());
        let led = emit(s, &map, &mut b, p);
        let ops = b.finish();
        let (q_peak, sha) = static_peak(s, &map, &ops);
        println!("static Q_peak {q_peak} ops sha256 {sha}");
        let t = if p.tw.item_outer {
            super::tables::SaTables::with_items(s, &map, p.tw.derive_id)
        } else {
            super::tables::SaTables::with(s, &map, p.tw.derive_id, p.tw.compact_outer)
        };
        println!(
            "== {id} swap {}: angle bits {} (outer data {}, inner word {}) u {} w {} rounding {err:.3e} ops {}",
            p.swap,
            t.angle_bits(),
            t.outer_data_bits(),
            t.inner_word_bits(),
            map.uniform_bits(),
            map.inner_width(),
            ops.len()
        );
        for r in &led.rows {
            println!("{:>8} {:>5}  {}", r.1, r.2, r.0);
        }
        let c = ledger_c_step(
            &led,
            u32::from(s.beta),
            map.uniform_bits(),
            map.inner_width(),
        );
        let swaps = if p.swap { 4.0 * (s.n + 1) as f64 } else { 0.0 };
        println!("static C_step {} (spin swaps {swaps})", c + swaps);
    }
}

/// The harness's static pass on `ops` (no lanes): `Q_peak` with the phase-gradient register,
/// and the op stream's SHA-256 (hex).
fn static_peak(
    s: &SaSpec,
    map: &crate::lanemap::sa_nested::SaNestedMap,
    ops: &[Op],
) -> (u64, String) {
    let file = ops_file(ops);
    let layout = crate::sim::Layout {
        system: s.system_qubits(),
        uniform: map.uniform_bits(),
    };
    let inner = map
        .inner_bits()
        .map(|(lo, width)| crate::sim::Inner { lo, width });
    let tr = givens_tracker(s);
    let c = crate::sim::compile_sa(&file.ops, &layout, tr, inner).unwrap();
    let grad = tr.map_or(0, |t| t.phase_gradient_qubits());
    (c.q_peak + grad, hex::encode(file.sha256))
}

/// Static per-stage ledgers of every `sa-toff` lever on the pinned specs (no harness run).
#[test]
#[ignore = "pinned specs; run in release"]
fn pinned_ledgers_toff() {
    let only = std::env::var("SA_TWEAKS").ok();
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let list: Vec<String> = only.clone().map_or_else(
            || TWEAKS.iter().map(ToString::to_string).collect(),
            |v| v.split(',').map(ToString::to_string).collect(),
        );
        for tw in list {
            println!("#### tweaks {tw}");
            pinned_ledger(
                id,
                s,
                Params {
                    tw: Tweaks::parse(&tw),
                    ..Params::for_spec(s)
                },
            );
        }
    }
}

/// For each outer keep width, the smallest inner keep width that passes the 0.1 mHa rule with
/// this walk's lane map (largest-remainder counts).
#[test]
#[ignore = "pinned specs; run in release"]
fn keep_bit_scan() {
    let limit = crate::spec::Exact::from_f64(1e-4).unwrap();
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let base = Params::for_spec(s);
        for mu_o in 14..=31u32 {
            let mut found = None;
            for mu_i in 10..=31u32 {
                if base.outer.0 + mu_o + base.inner.0 + mu_i + 2 > 63 {
                    break;
                }
                let p = Params {
                    outer: (base.outer.0, mu_o),
                    inner: (base.inner.0, mu_i),
                    ..base
                };
                let err = lane_map(s, p).unwrap().rounding_error(s).unwrap();
                if err <= limit {
                    found = Some((mu_i, err.to_f64()));
                    break;
                }
            }
            println!("{id} mu_o {mu_o}: {found:?}");
        }
    }
}

#[test]
#[ignore = "diagnostic"]
fn angle_diag() {
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let beta = u32::from(s.beta);
        let q = 1u32 << (beta - 2);
        let chain: Vec<(u16, u16)> = s.e_nets[0].rotations.iter().map(|r| (r.0, r.1)).collect();
        println!(
            "{id} beta {beta} chain head {:?} tail {:?}",
            &chain[..4],
            &chain[chain.len() - 3..]
        );
        let mut hist = vec![[0usize; 4]; chain.len()];
        for n in s.e_nets.iter().chain(&s.nets) {
            for (j, r) in n.rotations.iter().enumerate() {
                hist[j][(r.2 % (1 << beta) / q) as usize] += 1;
            }
        }
        for (j, h) in hist.iter().enumerate() {
            if j < 3 || j + 3 > chain.len() || h[2] + h[3] > 0 {
                println!("  rot {j}: quadrants {h:?}");
            }
        }
    }
}

/// Static `C_step` (ledger plus spin swaps) and `Q_peak` (the harness's static pass) of the
/// pinned spec under `p`: no lanes are run.
fn static_point(s: &SaSpec, p: Params) -> (f64, u64) {
    let map = lane_map(s, p).unwrap();
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let led = emit(s, &map, &mut b, p);
    let ops = b.finish();
    let (q, _) = static_peak(s, &map, &ops);
    let c = ledger_c_step(
        &led,
        u32::from(s.beta),
        map.uniform_bits(),
        map.inner_width(),
    );
    let swaps = if p.swap { 4.0 * (s.n + 1) as f64 } else { 0.0 };
    (c + swaps, q)
}

/// `sa-lowq`'s static frontier: `C_step`, `Q_peak` and their product for each chunk count and
/// inner QROAM block count (`FEMOCO_SA_CHUNKS`, `FEMOCO_SA_INNER_A`).
#[test]
#[ignore = "pinned specs; run in release"]
fn lowq_scan() {
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let base = Params::for_spec(s);
        for (outer_erase, dense) in [(false, false), (true, false), (true, true)] {
            for chunks in 1..=6 {
                for inner_a in [base.inner_a, base.inner_a - 1, base.inner_a - 2] {
                    let p = Params {
                        chunks,
                        inner_a,
                        outer_erase,
                        dense,
                        ..base
                    };
                    let (c, q) = static_point(s, p);
                    println!(
                        "{id} erase {outer_erase} dense {dense} C {chunks} inner_a {inner_a}: C_step {c} Q_peak {q} product {:.4e}",
                        c * q as f64
                    );
                }
            }
        }
    }
}

/// A quick harness run on the pinned specs at a small sample count (`SA_SAMPLES`, default
/// 2^10) for the levers in `SA_TWEAKS` (default `all`): structure, `C_step` and `Q_peak` on
/// the real data before a full `./benchmark.sh`. Not a measurement at the benchmark's `K`.
#[test]
#[ignore = "pinned specs; run in release"]
fn pinned_quick_eval() {
    TOFF.with(|c| c.set(true));
    let tw = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "all".into());
    let samples: usize = std::env::var("SA_SAMPLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1 << 10);
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-v1,li-sa-v1".into());
    for id in ids.split(',') {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let base = Params::for_spec(s);
        // Runtime overrides of the build-time knobs, for scans (`SA_INNER_A`, `SA_OUTER_A`).
        let rt = |k: &str, d: usize| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        let p = Params {
            tw: Tweaks::parse(&tw),
            inner_a: rt("SA_INNER_A", base.inner_a),
            outer_a: rt("SA_OUTER_A", base.outer_a),
            ..base
        };
        let (lm, ops, _) = build(s, p);
        let t0 = std::time::Instant::now();
        match eval(s, &lm, &ops, samples) {
            Ok(ev) => println!(
                "{id} {tw} ia={} oa={}: OK C_step {} Q_peak {} rounding {:.4e} ({} lanes, {:.1} s)",
                p.inner_a,
                p.outer_a,
                ev.toffoli,
                ev.qubits,
                ev.rounding_error.to_f64(),
                samples,
                t0.elapsed().as_secs_f64()
            ),
            Err(e) => panic!("{id} {tw}: {e}"),
        }
    }
}

/// The published walk (`sa-low2025`, `sa-low2025-bothspin`) emits exactly the op stream and lane
/// map of the measured replication of Low et al. 2025, whatever
/// `sa-pareto` adds.
#[test]
#[ignore = "pinned specs; run in release"]
fn legacy_ops_digest() {
    use sha2::{Digest, Sha256};
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for swap in [false, true] {
            let p = Params {
                swap,
                ..Params::for_spec(s)
            };
            let (lm, ops, _) = build(s, p);
            let mut h = Sha256::new();
            for o in &ops {
                h.update(format!("{o:?}").as_bytes());
            }
            h.update(&lm);
            let got = hex::encode(h.finalize());
            let want = match (id, swap) {
                ("reiher-sa-v1", false) => {
                    "a16e9a307df1d39e3e995bb4142a0cafc39584e313e619c4fed66bbeeb811f44"
                }
                ("reiher-sa-v1", true) => {
                    "af39f90a3d72c32b73f39af90e2c3613956f27ace96b7a4962a9817bfe287483"
                }
                ("li-sa-v1", false) => {
                    "f00619f7d87e2843f33a6bf257e98714d59267ddbf41c32fcb76d5f573133d7d"
                }
                _ => "1f28be300dbe8283b1d0c16db5267e6e67c82a694d2af6dcf6540323faf00a3f",
            };
            assert_eq!(
                got, want,
                "{id} swap {swap}: the published walk's ops changed"
            );
        }
    }
}

/// `sa-pareto`'s variants on the exact N = 3 spec: every combination passes with verified axes,
/// and the ledger still adds up to the harness's `C_step`.
#[test]
fn pareto_variants_pass() {
    for (lean, carries, chunks, drop_alt) in [
        (true, 0, 1, false),
        (false, 3, 1, true),
        (true, 1, 1, true),
        (true, 2, 2, false),
        (false, 0, 2, false),
        (true, 3, 2, true),
    ] {
        passes_on_an_exact_spec(Params {
            swap: true,
            pareto: true,
            lean,
            carries,
            chunks,
            drop_alt,
            ..P3
        });
    }
}

/// The variants on the padded N = 4 spec (three rotations, so up to three chunks; padding
/// buckets, one-body rows past `2^k_i`) with keep widths wide enough to pass the rounding rule.
#[test]
fn pareto_variants_pass_padded() {
    let s = spec4();
    for (lean, carries, chunks, oa, ia, drop_alt) in [
        (true, 3, 1, 2, 1, false),
        (true, 2, 2, 0, 2, true),
        (true, 0, 3, 3, 0, true),
        (false, 1, 3, 1, 1, false),
        (true, 1, 2, 1, 2, true),
    ] {
        let p = Params {
            outer: (3, 28),
            inner: (2, 28),
            outer_a: oa,
            inner_a: ia,
            swap: true,
            pareto: true,
            lean,
            carries,
            chunks,
            drop_alt,
            ..P3
        };
        let (lm, ops, led) = build(&s, p);
        let ev = eval(&s, &lm, &ops, 1 << 13).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        let map = lane_map(&s, p).unwrap();
        let want =
            ledger_c_step(&led, BETA, map.uniform_bits(), map.inner_width()) + ev.spin_swap_toffoli;
        assert!(
            (ev.toffoli - want).abs() < 1e-9,
            "{p:?}: {} vs {want}",
            ev.toffoli
        );
    }
}

/// Mutants of the lean, chunked walk are rejected.
#[test]
fn pareto_mutants_are_rejected() {
    mutants_rejected(Params {
        swap: true,
        pareto: true,
        lean: true,
        carries: 3,
        chunks: 2,
        drop_alt: true,
        ..P3
    });
}

/// Static `C_step` (ledger) and `Q_peak` (the harness's own static pass, `compile_sa`, plus the
/// phase-gradient register) of `sa-pareto` points on the pinned specs, no lane simulation.
/// Points: `FEMOCO_SA_SCAN="chunks,inner_a,outer_a,lean,carries;..."` at run time (default: a
/// small grid). These are static counts for choosing points; the measured numbers are
/// `./benchmark.sh`'s.
#[test]
#[ignore = "pinned specs; run in release"]
fn pareto_scan() {
    use crate::sim::{compile_sa, Inner, Layout};
    let grid = std::env::var("FEMOCO_SA_SCAN").unwrap_or_else(|_| {
        "1,-1,2,0,0;1,-1,2,1,0;1,-1,2,1,1;2,-1,2,1,1;3,-1,2,1,1;4,-1,2,1,1;4,3,2,1,1".into()
    });
    let ids: Vec<String> = std::env::var("FEMOCO_SA_SCAN_SPECS")
        .unwrap_or_else(|_| "reiher-sa-v1,li-sa-v1".into())
        .split(',')
        .map(str::to_string)
        .collect();
    for id in &ids {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for pt in grid.split(';') {
            let v: Vec<i64> = pt.split(',').map(|x| x.trim().parse().unwrap()).collect();
            let base = Params::for_spec(s);
            let p = Params {
                pareto: true,
                chunks: v[0] as usize,
                inner_a: if v[1] < 0 {
                    base.inner_a
                } else {
                    v[1] as usize
                },
                outer_a: v[2] as usize,
                lean: v[3] != 0,
                carries: v[4] as u32,
                drop_alt: v.get(5).is_some_and(|&d| d != 0),
                ..base
            };
            let map = lane_map(s, p).unwrap();
            let mut b = Builder::new(2 * s.n);
            b.declare_uniform(map.uniform_bits());
            let led = emit(s, &map, &mut b, p);
            let ops = b.finish();
            let layout = Layout {
                system: 2 * s.n,
                uniform: map.uniform_bits(),
            };
            let inner = Some(Inner {
                lo: map.outer_bits(),
                width: map.inner_width(),
            });
            let c = compile_sa(&ops, &layout, givens_tracker(s), inner).unwrap();
            let q = c.q_peak + u64::from(s.beta);
            let t = ledger_c_step(
                &led,
                u32::from(s.beta),
                map.uniform_bits(),
                map.inner_width(),
            ) + 4.0 * (s.n + 1) as f64;
            println!(
                "SCAN {id} C={} ia={} oa={} lean={} carries={} drop_alt={}: C_step {t} Q_peak {q} product {:.4e}",
                p.chunks,
                p.inner_a,
                p.outer_a,
                p.lean,
                p.carries,
                p.drop_alt,
                t * q as f64
            );
            if std::env::var("FEMOCO_SA_SCAN_LEDGER").is_ok() {
                for r in &led.rows {
                    println!("{:>8} {:>5}  {}", r.1, r.2, r.0);
                }
            }
        }
    }
}

/// Rounding errors of chosen keep splits: `FEMOCO_SA_KEEPS="mu_o,mu_i;..."` at run time.
#[test]
#[ignore = "pinned specs; run in release"]
fn keep_errors() {
    let pts = std::env::var("FEMOCO_SA_KEEPS").unwrap_or_else(|_| "19,19;19,18;20,18;22,18".into());
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let base = Params::for_spec(s);
        for pt in pts.split(';') {
            let v: Vec<u32> = pt.split(',').map(|x| x.trim().parse().unwrap()).collect();
            let p = Params {
                outer: (base.outer.0, v[0]),
                inner: (base.inner.0, v[1]),
                ..base
            };
            let err = lane_map(s, p).unwrap().rounding_error(s).unwrap().to_f64();
            println!("KEEP {id} ({}, {}): {err:.4e}", v[0], v[1]);
        }
    }
}

/// `G = E_up - E_SOS` from the pinned spec's certificate (`sos-ground-cs-v1`, spec/SPEC-SA.md
/// section 12).
fn certificate_gap(id: &str, s: &SaSpec) -> Exact {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cert: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(format!("specs/{id}/certificate.json"))).unwrap(),
    )
    .unwrap();
    let e_up = cert["ground_energy_upper_bound"]["E_up"].as_f64().unwrap();
    Exact::from_f64(e_up).unwrap().sub(&s.e_sos)
}

/// For each outer keep width, the smallest inner keep width whose lane map meets
/// each budget in `SA_RIG_BUDGETS` (mHa, default `0.1,0.125,0.25`) under the ground-energy
/// bound, and the static `C_step` / `Q_peak` of the walks in `SA_RIG_WALKS` there
/// (`toff:<letters>` or `pareto:<chunks>,<inner_a>,<outer_a>,<carries>,<drop_alt>`; default
/// sa-toff `all` and `imchxL`). Specs: `SA_RIG_SPECS` (default both sa-v1 specs). With
/// `SA_RIG_ONE_NORM=1` the 1-norm rounding error is held to the budget instead (the budget lever
/// alone).
#[test]
#[ignore = "pinned specs; run in release"]
fn rigbits_scan() {
    let budgets: Vec<f64> = std::env::var("SA_RIG_BUDGETS")
        .unwrap_or_else(|_| "0.1,0.125,0.25".into())
        .split(',')
        .map(|x| x.parse().unwrap())
        .collect();
    let walks: Vec<String> = std::env::var("SA_RIG_WALKS")
        .unwrap_or_else(|_| "toff:all,toff:imchxL".into())
        .split(';')
        .flat_map(|x| x.split(",toff").map(str::to_string).collect::<Vec<_>>())
        .map(|x| {
            if x.starts_with(':') {
                format!("toff{x}")
            } else {
                x
            }
        })
        .collect();
    let ids = std::env::var("SA_RIG_SPECS").unwrap_or_else(|_| "reiher-sa-v1,li-sa-v1".into());
    let mu_os: Vec<u32> = std::env::var("SA_RIG_MU_O").map_or_else(
        |_| (14..=22).collect(),
        |v| v.split(',').map(|x| x.parse().unwrap()).collect(),
    );
    for id in ids.split(',') {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let g = certificate_gap(id, s);
        let base = Params::for_spec(s);
        for &mh in &budgets {
            let budget = Exact::from_f64(mh / 1000.0).unwrap();
            for &mu_o in &mu_os {
                let found = (8..=24u32).find_map(|mu_i| {
                    let p = Params {
                        outer: (base.outer.0, mu_o),
                        inner: (base.inner.0, mu_i),
                        ..base
                    };
                    let map = lane_map(s, p).unwrap();
                    if std::env::var("SA_RIG_ONE_NORM").is_ok() {
                        let e = map.rounding_error(s).unwrap();
                        return (e <= budget).then_some((mu_i, e.to_f64()));
                    }
                    let b = map.ground_bound(s, &g).unwrap();
                    b.within(&budget).then_some((mu_i, b.bound.to_f64()))
                });
                let Some((mu_i, bound)) = found else {
                    println!("RIG {id} budget {mh} mHa mu_o {mu_o}: none");
                    continue;
                };
                for wk in &walks {
                    let (kind, arg) = wk.split_once(':').unwrap();
                    let mut p = Params {
                        outer: (base.outer.0, mu_o),
                        inner: (base.inner.0, mu_i),
                        ..base
                    };
                    if kind == "toff" {
                        p.tw = Tweaks::parse(arg);
                    } else {
                        let v: Vec<usize> = arg.split(',').map(|x| x.parse().unwrap()).collect();
                        p = Params {
                            pareto: true,
                            lean: true,
                            chunks: v[0],
                            inner_a: v[1],
                            outer_a: v[2],
                            carries: v[3] as u32,
                            drop_alt: v[4] != 0,
                            ..p
                        };
                    }
                    let (c, q) = static_point(s, p);
                    println!(
                        "RIG {id} budget {mh} mHa ({mu_o}, {mu_i}) bound {bound:.4e} {wk}: C_step {c} Q_peak {q}"
                    );
                }
            }
        }
    }
}

/// Static `C_step` / `Q_peak` of any architecture on any sos-sa spec under any root, all set at
/// run time (spec/SPEC-SA.md section 14). No lanes are run; the
/// measured numbers are `./benchmark.sh`'s.
/// - `SA_ROOT` (default this checkout; or a scratch root holding other specs),
///   `SA_SPECS` (comma-separated ids);
/// - `SA_ARCH` = `toff` (default; `SA_TWEAKS`, default `imchxL`), `pareto` or `lowq`;
/// - overrides `SA_MU=o,i`, `SA_INNER_A`, `SA_OUTER_A`, `SA_CHUNKS`, `SA_CARRIES`,
///   `SA_DROP_ALT`, `SA_OUTER_ERASE`, `SA_DENSE`, `SA_INNER_ALL` (the write-all-words inner lookup,
///   `range.rs`); `SA_LEDGER=1` prints the per-stage rows.
#[test]
#[ignore = "pinned or scratch specs; run in release"]
fn static_any() {
    let root = std::env::var("SA_ROOT").unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").into());
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "reiher-sa-est-v1".into());
    let arch = std::env::var("SA_ARCH").unwrap_or_else(|_| "toff".into());
    let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<i64>().ok());
    for id in ids.split(',') {
        let boxed = crate::spec::load(std::path::Path::new(&root), id).unwrap();
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let base = Params::for_spec(s);
        let mut p = match arch.as_str() {
            "toff" => Params {
                tw: Tweaks::parse(&std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxL".into())),
                ..base
            },
            "pareto" => Params {
                pareto: true,
                lean: true,
                ..base
            },
            "lowq" => Params {
                chunks: 2,
                inner_a: base.inner_a.min(4),
                outer_erase: true,
                dense: true,
                ..base
            },
            other => panic!("SA_ARCH {other}"),
        };
        if let Ok(v) = std::env::var("SA_MU") {
            let m: Vec<u32> = v.split(',').map(|x| x.trim().parse().unwrap()).collect();
            p.outer.1 = m[0];
            p.inner.1 = m[1];
        }
        if let Some(v) = get("SA_INNER_A") {
            p.inner_a = v as usize;
        }
        if let Some(v) = get("SA_OUTER_A") {
            p.outer_a = v as usize;
        }
        if let Some(v) = get("SA_CHUNKS") {
            p.chunks = v as usize;
        }
        if let Some(v) = get("SA_CARRIES") {
            p.carries = v as u32;
        }
        if let Some(v) = get("SA_DROP_ALT") {
            p.drop_alt = v != 0;
        }
        if let Some(v) = get("SA_OUTER_ERASE") {
            p.outer_erase = v != 0;
        }
        if let Some(v) = get("SA_DENSE") {
            p.dense = v != 0;
        }
        let map = lane_map(s, p).unwrap();
        let mut b = Builder::new(2 * s.n);
        b.declare_uniform(map.uniform_bits());
        let led = emit(s, &map, &mut b, p);
        let ops = b.finish();
        let (q, sha) = static_peak(s, &map, &ops);
        let c = ledger_c_step(
            &led,
            u32::from(s.beta),
            map.uniform_bits(),
            map.inner_width(),
        ) + if p.swap { 4.0 * (s.n + 1) as f64 } else { 0.0 };
        println!(
            "STATIC {root}:{id} (R,B,C)=({},{},{}) beta {} arch {arch} mu ({},{}) ia {} oa {} C {} carries {} drop_alt {}: C_step {c} Q_peak {q} ops {}",
            s.r,
            s.b,
            s.c,
            s.beta,
            p.outer.1,
            p.inner.1,
            p.inner_a,
            p.outer_a,
            p.chunks,
            p.carries,
            p.drop_alt,
            &sha[..16]
        );
        if std::env::var("SA_LEDGER").is_ok() {
            for r in &led.rows {
                println!("{:>8} {:>5}  {}", r.1, r.2, r.0);
            }
        }
    }
}

/// N = 5, (R, B, C) = (1, 3, 1): one-body weights 8, 4, 4, 4, 4 and one square with four equal
/// inner weights (S^2 / 2 = 8; outer total 32, exact). With `k_i = 3` its 4 inner items sit in 8
/// buckets, and no item holds half the lanes, so lever `p` needs two word classes (`d = 1`).
fn spec6() -> SaSpec {
    let bytes = payload(
        5,
        (1, 3, 1),
        &[4.0, -2.0, 2.0, 2.0, 2.0],
        &[1.0],
        &[1.0, -1.0, 1.0],
    );
    parse_payload("test-sa6-v1", &bytes).unwrap()
}

/// Lever `p` (`range.rs`, the padded write-all-words inner read) on exact specs whose inner
/// tables have padding (`k_i = 3`): the walk passes the harness, the plan is the expected one,
/// and each of the two inner reads per step saves exactly `PadPlan::saved() x w` Toffolis
/// against the same levers without `p`, on both erase paths (measured undo or not).
#[test]
fn toff_pad_share_passes() {
    TOFF.with(|c| c.set(true));
    for (s, want) in [(spec3(), (2, 0)), (spec6(), (2, 1))] {
        for tw in ["p", "xp", "imchxp", "imchxLp", "imchxlp"] {
            for swap in [true, false] {
                let p = Params {
                    inner: (3, 1),
                    inner_a: 3,
                    swap,
                    tw: Tweaks::parse(tw),
                    ..P3
                };
                let map = lane_map(&s, p).unwrap();
                let plan = super::pad_plan_of(&s, p, &map.inner).expect("a pad plan");
                assert_eq!((plan.s, plan.d), want, "{} {tw}", s.id());
                println!("== {} tweaks {tw} swap {swap} plan {plan:?}", s.id());
                passes_on(&s, p);
                let q = Params {
                    tw: Tweaks::parse(&tw.replace('p', "")),
                    ..p
                };
                let (_, _, lp) = build(&s, p);
                let (_, _, lq) = build(&s, q);
                let w =
                    super::tables::SaTables::with_items(&s, &map, p.tw.derive_id).inner_word_bits();
                let tw_of = |l: &super::ledger::Ledger| 2 * l.sum("copy").0 + l.sum("outer").0;
                assert_eq!(
                    tw_of(&lq) - tw_of(&lp),
                    (2 * plan.saved()
                        * if p.tw.item_outer {
                            w
                        } else {
                            super::tables::SaTables::with(
                                &s,
                                &map,
                                p.tw.derive_id,
                                p.tw.compact_outer,
                            )
                            .inner_word_bits()
                        }) as u64,
                    "{} {tw}",
                    s.id()
                );
            }
        }
    }
}

/// Without `p`, or with fewer than `2^k_i` inner blocks, the walk is the one it was.
#[test]
fn pad_share_off_is_unchanged() {
    let s = spec6();
    for tw in ["imchxL", "imchx", "x"] {
        let p = Params {
            inner: (3, 1),
            inner_a: 3,
            tw: Tweaks::parse(tw),
            ..P3
        };
        let map = lane_map(&s, p).unwrap();
        assert!(super::pad_plan_of(&s, p, &map.inner).is_none());
        // `p` with inner_a < k_i: no plan, the same op stream as without `p`.
        let with_p = Params {
            inner_a: 2,
            tw: Tweaks::parse(&format!("{tw}p")),
            ..p
        };
        let without = Params { inner_a: 2, ..p };
        assert_eq!(build(&s, with_p).1, build(&s, without).1, "{tw}");
    }
    assert!(!Tweaks::parse("all").pad_share);
}

/// `sa-pareto` points from a scan string `chunks,inner_a,outer_a,lean,carries,drop_alt[,...]`
/// on `s` (`Params::for_spec`'s keep split), as `pareto_scan` reads them. Fields past the
/// sixth are `stream_narrow`'s knobs (`pareto.rs`, `Narrow::parse`), as one letter string.
fn scan_point(s: &SaSpec, pt: &str) -> Params {
    let v: Vec<&str> = pt.split(',').map(str::trim).collect();
    let n = |i: usize| v[i].parse::<i64>().unwrap();
    let base = Params::for_spec(s);
    Params {
        pareto: true,
        chunks: n(0) as usize,
        inner_a: if n(1) < 0 {
            base.inner_a
        } else {
            n(1) as usize
        },
        outer_a: n(2) as usize,
        lean: n(3) != 0,
        carries: n(4) as u32,
        drop_alt: v.get(5).is_some_and(|d| *d != "0"),
        narrow: super::pareto::Narrow::parse(v.get(6).copied().unwrap_or("")),
        ..base
    }
}

/// Peak census of the emitted stream (the harness's liveness rule, `src/sim/liveness.rs`):
/// every ancilla is tagged with the ledger stage in which it is first touched, and at the
/// stream's peak the live ancillas are counted by tag; also the largest live count inside
/// each stage (the peak families). Points: `FEMOCO_SA_CENSUS="spec|point;..."`, a point as
/// [`scan_point`] reads it. Static, no lanes.
#[test]
#[ignore = "pinned specs; run in release"]
fn stream_census() {
    let pts = std::env::var("FEMOCO_SA_CENSUS")
        .unwrap_or_else(|_| "reiher-sa-gb-r15-v1|8,2,1,1,0,1".into());
    for item in pts.split(';') {
        let (id, pt) = item.split_once('|').unwrap();
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = scan_point(s, pt);
        let map = lane_map(s, p).unwrap();
        let mut b = Builder::new(2 * s.n);
        b.declare_uniform(map.uniform_bits());
        let led = emit(s, &map, &mut b, p);
        let ops = b.finish();
        let (q_peak, _) = static_peak(s, &map, &ops);
        let first = 1 + 2 * s.n as u32 + map.uniform_bits();
        let persistent = u64::from(first) + u64::from(s.beta);
        // The ledger's rows in stream order (the copy's rows twice), as (Toffoli, Givens) bounds.
        let copy_rows: Vec<_> = led
            .rows
            .iter()
            .filter(|r| r.0.starts_with("copy"))
            .cloned()
            .collect();
        let mut seq = Vec::new();
        let mut done = false;
        for r in &led.rows {
            if r.0.starts_with("copy") {
                if !done {
                    seq.extend(copy_rows.iter().cloned());
                    seq.extend(copy_rows.iter().cloned());
                    done = true;
                }
            } else {
                seq.push(r.clone());
            }
        }
        let mut bounds = Vec::new();
        let (mut t, mut g) = (0u64, 0u64);
        for r in &seq {
            t += r.1;
            g += r.2;
            bounds.push((t, g));
        }
        let mut regs: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
        let mut live: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
        let mut depth = 0usize;
        let (mut ct, mut cg, mut si) = (0u64, 0u64, 0usize);
        let mut stage_peak = vec![0usize; seq.len()];
        let (mut peak, mut at_peak, mut peak_stage) = (0usize, Vec::new(), 0usize);
        for o in &ops {
            if matches!(o.kind, K::CCX | K::CCZ) {
                ct += 1;
            }
            if o.kind == K::Givens {
                cg += 1;
            }
            while si + 1 < bounds.len() && (ct > bounds[si].0 || cg > bounds[si].1) {
                si += 1;
            }
            match o.kind {
                K::PushCondition => depth += 1,
                K::PopCondition => depth -= 1,
                K::Register => {
                    regs.insert(o.r_target, Vec::new());
                }
                K::AppendToRegister => regs.entry(o.r_target).or_default().push(o.q_target),
                K::DebugPrint | K::Segment => {}
                K::R | K::Hmr if depth == 0 && o.c_condition == crate::circuit::NONE => {
                    live.remove(&o.q_target);
                }
                K::Givens => {
                    for &q in regs.get(&o.r_target).into_iter().flatten() {
                        live.entry(q).or_insert(si);
                    }
                    for q in o.qubits() {
                        if q >= first {
                            live.entry(q).or_insert(si);
                        }
                    }
                }
                _ => {
                    for q in o.qubits() {
                        if q >= first {
                            live.entry(q).or_insert(si);
                        }
                    }
                }
            }
            stage_peak[si] = stage_peak[si].max(live.len());
            if live.len() > peak {
                peak = live.len();
                peak_stage = si;
                at_peak = live.values().copied().collect();
            }
        }
        println!(
            "CENSUS {id} {pt}: Q_peak {q_peak} (static pass) = persistent {persistent} + {peak} at '{}'",
            seq[peak_stage].0
        );
        let mut by: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        for s in at_peak {
            *by.entry(s).or_insert(0) += 1;
        }
        for (s, n) in by {
            println!("   {n:>5}  first touched in #{s} {}", seq[s].0);
        }
        let mut fam: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for (i, r) in seq.iter().enumerate() {
            let e = fam.entry(r.0.as_str()).or_insert(0);
            *e = (*e).max(stage_peak[i]);
        }
        let mut v: Vec<_> = fam.into_iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        for (k, m) in v.iter().take(8) {
            println!("   family {:>5} {k}", persistent + *m as u64);
        }
    }
}

/// Expected executed Toffolis of `ops`: each `CCX`/`CCZ` weighted by `2^-d`, `d` the condition
/// bits it sits under (every conditioned Toffoli the walks emit is gated on fair X-basis
/// outcomes), and the Toffolis inside condition blocks at full weight.
fn expected_toffolis(ops: &[Op]) -> (f64, u64) {
    let (mut depth, mut exp, mut gated) = (0i32, 0.0, 0u64);
    for o in ops {
        match o.kind {
            K::PushCondition => depth += 1,
            K::PopCondition => depth -= 1,
            K::CCX | K::CCZ => {
                let d = depth + i32::from(o.c_condition != crate::circuit::NONE);
                exp += 0.5f64.powi(d);
                if d > 0 {
                    gated += 1;
                }
            }
            _ => {}
        }
    }
    (exp, gated)
}

/// The `pareto::Narrow` gadgets on the exact N = 3 spec and the padded N = 4 spec: every
/// combination passes the harness with verified axes, and the harness's `C_step` is the static
/// ledger's with each gated Toffoli counted at its execution probability (to sampling error).
#[test]
fn narrow_variants_pass() {
    use super::pareto::Narrow;
    let cases: Vec<(SaSpec, Params)> = vec![
        (
            spec3(),
            Params {
                swap: true,
                pareto: true,
                lean: true,
                chunks: 1,
                drop_alt: true,
                ..P3
            },
        ),
        (
            spec3(),
            Params {
                swap: true,
                pareto: true,
                lean: true,
                chunks: 2,
                drop_alt: true,
                ..P3
            },
        ),
        (
            spec5(),
            Params {
                swap: true,
                pareto: true,
                lean: true,
                chunks: 2,
                drop_alt: true,
                outer: (4, 1),
                inner: (2, 2),
                ..P3
            },
        ),
        (
            spec5(),
            Params {
                swap: true,
                pareto: true,
                lean: true,
                chunks: 4,
                drop_alt: true,
                outer: (4, 1),
                inner: (2, 2),
                ..P3
            },
        ),
        (
            spec4(),
            Params {
                swap: true,
                pareto: true,
                lean: true,
                chunks: 2,
                drop_alt: true,
                outer: (3, 28),
                inner: (2, 28),
                outer_a: 1,
                inner_a: 2,
                ..P3
            },
        ),
        (
            spec4(),
            Params {
                swap: true,
                pareto: true,
                lean: true,
                chunks: 3,
                drop_alt: true,
                outer: (3, 28),
                inner: (2, 28),
                outer_a: 3,
                inner_a: 0,
                ..P3
            },
        ),
    ];
    for (s, base) in cases {
        for nw in [
            "e",
            "k",
            "r",
            "g",
            "c",
            "ek",
            "kr0",
            "ek1r1",
            "ek3r2",
            "ek0r0",
            "egc",
            "ecgk",
            "ecgkr1",
            "kra",
            "egckr1a",
            "egckr0ah2",
            "ra",
            "ep",
            "ej",
            "epj",
            "egckpj",
            "egckr1apj",
            "eko",
            "egckopj",
            "egckr1apjo",
            "ekq",
            "egckpjq1",
            "egckr1apjq",
        ] {
            let p = Params {
                narrow: Narrow::parse(nw),
                ..base
            };
            let (lm, ops, led) = build(&s, p);
            let ev = eval(&s, &lm, &ops, 1 << 14)
                .unwrap_or_else(|e| panic!("{} {nw} {p:?}: {e}", s.id()));
            for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                assert_eq!(v.status, AxisStatus::Verified, "{axis}");
            }
            let map = lane_map(&s, p).unwrap();
            let static_c = ledger_c_step(&led, BETA, map.uniform_bits(), map.inner_width())
                + ev.spin_swap_toffoli;
            let (exp, gated) = expected_toffolis(&ops);
            let all = ops
                .iter()
                .filter(|o| matches!(o.kind, K::CCX | K::CCZ))
                .count() as f64;
            let want = static_c - all + exp;
            // Sampling error of the gated blocks' fair coins: at most sqrt(gated^2 / 4 / K) per
            // lane average; allow 6 sigma plus rounding.
            let tol = 6.0 * (gated as f64) / (2.0 * f64::from(1u32 << 7)) + 1e-9;
            assert!(
                (ev.toffoli - want).abs() <= tol,
                "{} {nw}: harness {} vs expected {want} (gated {gated})",
                s.id(),
                ev.toffoli
            );
            println!(
                "{} C {} {nw}: harness C_step {:.3} expected {want:.3} static {static_c} Q {}",
                s.id(),
                p.chunks,
                ev.toffoli,
                ev.qubits
            );
        }
    }
}

/// Mutants of the gated re-read are rejected: dropping the conditioned `CZ` (the comparison's
/// top carry phase) or the conditioned `Z` in every gated block, and the generic mutants.
#[test]
fn narrow_mutants_are_rejected() {
    use super::pareto::Narrow;
    let s = spec4();
    let p = Params {
        swap: true,
        pareto: true,
        lean: true,
        chunks: 2,
        drop_alt: true,
        outer: (3, 28),
        inner: (2, 28),
        outer_a: 1,
        inner_a: 1,
        narrow: Narrow::parse("ekrcapj"),
        ..P3
    };
    let (lm, ops, _) = build(&s, p);
    eval(&s, &lm, &ops, 1 << 12).expect("the unmutated walk passes");
    let drop_in_blocks = |kind: K| -> Vec<Op> {
        let mut depth = 0;
        let mut n = 0;
        let out: Vec<Op> = ops
            .iter()
            .filter(|o| {
                match o.kind {
                    K::PushCondition => depth += 1,
                    K::PopCondition => depth -= 1,
                    _ => {}
                }
                let hit = depth > 0 && o.kind == kind && o.c_condition == crate::circuit::NONE;
                n += usize::from(hit);
                !hit
            })
            .copied()
            .collect();
        assert!(n > 0, "no {kind:?} inside a gated block");
        out
    };
    // The same, inside the inner copies only (between Segment 3 and Segment 4).
    let drop_inner = |kind: K| -> Vec<Op> {
        let (mut depth, mut inner, mut n) = (0, false, 0);
        let out: Vec<Op> = ops
            .iter()
            .filter(|o| {
                match o.kind {
                    K::PushCondition => depth += 1,
                    K::PopCondition => depth -= 1,
                    K::Segment if o.r_target == crate::circuit::SEG_INNER_BEGIN => inner = true,
                    K::Segment if o.r_target == crate::circuit::SEG_INNER_END => inner = false,
                    _ => {}
                }
                let hit =
                    inner && depth > 0 && o.kind == kind && o.c_condition == crate::circuit::NONE;
                n += usize::from(hit);
                !hit
            })
            .copied()
            .collect();
        assert!(n > 0, "no {kind:?} inside an inner gated block");
        out
    };
    for (name, m) in [
        ("gated CZ", drop_in_blocks(K::CZ)),
        ("gated Z", drop_in_blocks(K::Z)),
        ("inner gated CZ", drop_inner(K::CZ)),
    ] {
        let e = eval(&s, &lm, &m, 1 << 12);
        assert!(e.is_err(), "{name} mutant must be rejected");
        println!("{name}: {}", e.err().unwrap());
    }
    // (The inner gated `Z` alone is not a mutant on this spec: its inner weights are dyadic,
    // so every keep has zero low bits and the carry that `Z` reads is 0 on every lane.
    // `narrow::tests` drops it on random tables.)
    // `o`: without the recomputed lt's CZ with g, or without g's m_l part.
    let po = Params {
        narrow: Narrow::parse("ekrcapjo"),
        ..p
    };
    let (lmo, opso, _) = build(&s, po);
    eval(&s, &lmo, &opso, 1 << 12).expect("the unmutated o walk passes");
    let outside = |pick: &dyn Fn(&Op) -> bool| -> Vec<Op> {
        let (mut inner, mut n) = (false, 0);
        let out: Vec<Op> = opso
            .iter()
            .filter(|o| {
                if o.kind == K::Segment && o.r_target == crate::circuit::SEG_INNER_BEGIN {
                    inner = true;
                }
                if o.kind == K::Segment && o.r_target == crate::circuit::SEG_INNER_END {
                    inner = false;
                }
                let hit = !inner && pick(o);
                n += usize::from(hit);
                !hit
            })
            .copied()
            .collect();
        assert!(n > 0, "nothing to drop");
        out
    };
    let x_if = outside(&|o: &Op| o.kind == K::X && o.c_condition != crate::circuit::NONE);
    let unprep = opso
        .iter()
        .rposition(|o| o.kind == K::Segment && o.r_target == crate::circuit::SEG_UNPREPARE)
        .unwrap();
    let cz_at = unprep
        + opso[unprep..]
            .iter()
            .position(|o| o.kind == K::CZ && o.c_condition == crate::circuit::NONE)
            .unwrap();
    let mut no_cz = opso.clone();
    no_cz.remove(cz_at);
    for (name, m) in [("o: g without m_l", x_if), ("o: no CZ(lt, g)", no_cz)] {
        let e = eval(&s, &lmo, &m, 1 << 12);
        assert!(e.is_err(), "{name} mutant must be rejected");
        println!("{name}: {}", e.err().unwrap());
    }
    mutants_rejected(Params {
        outer: (3, 2),
        inner: (2, 1),
        outer_a: 1,
        inner_a: 1,
        chunks: 2,
        ..p
    });
}

/// Static expected `C_step` (ledger, every gated Toffoli at its execution probability, spin
/// swaps) and `Q_peak` (the harness's static pass) of `p` on `s`.
fn static_expected(s: &SaSpec, p: Params) -> (f64, u64) {
    let map = lane_map(s, p).unwrap();
    let mut b = Builder::new(2 * s.n);
    b.declare_uniform(map.uniform_bits());
    let led = emit(s, &map, &mut b, p);
    let ops = b.finish();
    let (q, _) = static_peak(s, &map, &ops);
    let c = ledger_c_step(
        &led,
        u32::from(s.beta),
        map.uniform_bits(),
        map.inner_width(),
    );
    let (exp, _) = expected_toffolis(&ops);
    let all = ops
        .iter()
        .filter(|o| matches!(o.kind, K::CCX | K::CCZ))
        .count() as f64;
    let swaps = if p.swap { 4.0 * (s.n + 1) as f64 } else { 0.0 };
    (c - all + exp + swaps, q)
}

/// The `pareto::Narrow` gadgets' static frontier: `FEMOCO_SA_NSCAN="spec|point;..."` (a point as
/// [`scan_point`] reads it, the seventh field the `FEMOCO_SA_NARROW` letters), or a grid:
/// `FEMOCO_SA_NGRID_SPECS`, `FEMOCO_SA_NGRID_C`, `_IA`, `_OA`, `_NW` (comma lists; `_NW` is a
/// `/`-separated list of letter strings). Prints `NSCAN spec point: C_step Q_peak`.
#[test]
#[ignore = "pinned specs; run in release"]
fn narrow_scan() {
    let list = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
    let mut pts: Vec<(String, String)> = Vec::new();
    if let Ok(v) = std::env::var("FEMOCO_SA_NSCAN") {
        for item in v.split(';') {
            let (id, pt) = item.split_once('|').unwrap();
            pts.push((id.into(), pt.into()));
        }
    } else {
        for id in list("FEMOCO_SA_NGRID_SPECS", "reiher-sa-est-v1").split(',') {
            for c in list("FEMOCO_SA_NGRID_C", "2,3,4,6,8").split(',') {
                for ia in list("FEMOCO_SA_NGRID_IA", "1,2,3,4").split(',') {
                    for oa in list("FEMOCO_SA_NGRID_OA", "1,2").split(',') {
                        for nw in list("FEMOCO_SA_NGRID_NW", "/e/ek").split('/') {
                            pts.push((id.into(), format!("{c},{ia},{oa},1,0,1,{nw}")));
                        }
                    }
                }
            }
        }
    }
    let mut cache: std::collections::HashMap<String, Box<dyn EncodingSpec>> =
        std::collections::HashMap::new();
    for (id, pt) in pts {
        let boxed = cache.entry(id.clone()).or_insert_with(|| pinned(&id));
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let p = scan_point(s, &pt);
        let (c, q) = static_expected(s, p);
        println!("NSCAN {id} {pt}: C_step {c:.2} Q_peak {q}");
    }
}
