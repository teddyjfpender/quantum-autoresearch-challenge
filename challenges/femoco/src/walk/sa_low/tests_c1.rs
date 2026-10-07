//! `sa-toff` levers `g` (outcome-gated comparator erasure) and `u` (one-hot RPREP) on small
//! sos-sa specs with dyadic weights, end to end through `score::evaluate`: every combination
//! passes with verified axes, the expected-count ledger matches the harness's `C_step` (exactly
//! without `g`, within sampling noise with it), and mutants of each new erasure are rejected.
//! The pinned-spec scan (`c1_scan`) is `#[ignore]`d; run it in release.
use super::onehot;
use super::{emit, family, family_toff, lane_map, Params, Tweaks};
use crate::circuit::{read_ops, write_ops, Builder, Op, OperationType as K, OpsFile, NONE};
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

/// `FEMOSAS1` for `n` orbitals and `(R, B, C)` (as `tests.rs`).
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

/// `tests.rs`'s exact N = 3 spec.
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

/// `tests.rs`'s exact N = 5 spec (four rotations per network).
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

/// `tests.rs`'s padded N = 4 spec (not exact: evaluate must fail only on the rounding rule).
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

/// Wider keep draws than `tests.rs`'s `P3` so the gated recompute has carries to skip.
const P: Params = Params {
    outer: (3, 5),
    inner: (2, 6),
    outer_a: 1,
    inner_a: 1,
    swap: true,
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

/// The lever bundles checked: each new lever alone, with the `sa-toff` default, with the held
/// ladders (where `g` has nothing left to gate on that side), and every lever that changes how
/// the network index is formed.
const TWEAKS: [&str; 26] = [
    "g",
    "u",
    "gu",
    "mu",
    "hu",
    "iu",
    "imchxg",
    "imchxu",
    "imchxgu",
    "imchxlgu",
    "imchxLu",
    "imchxLg",
    "r",
    "imchxgr",
    "imchxlgr",
    "ir",
    "d",
    "md",
    "imchxgrd",
    "imchxLrd",
    "v",
    "w",
    "rv",
    "imchxgrv",
    "imchxgrw",
    "imchxgrvd",
];

static TMP: AtomicU64 = AtomicU64::new(0);

fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-sa-c1-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

fn eval(
    s: &SaSpec,
    p: Params,
    lanemap: &[u8],
    ops: &[Op],
    samples: usize,
) -> Result<Evaluation, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    let tax = taxonomy::load_taxonomy(&path).unwrap();
    let check = |f: &_, facts: &_| taxonomy::check(&tax, f, facts);
    let fam = serde_json::to_vec(&FamilyOut {
        family: if p.tw.any_toff() {
            family_toff()
        } else {
            family(p.swap)
        },
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

/// Expected `C_step` from the ledger (the copy twice, Givens at `2 (beta - 2)`, reflections,
/// spin swaps) and the variance of one lane's gated Toffolis (for the sampling tolerance).
fn ledger_c_step(led: &super::ledger::Ledger, s: &SaSpec, p: Params) -> (f64, f64) {
    let map = lane_map(s, p).unwrap();
    let (to, go) = led.expected_sum("outer");
    let (tc, gc) = led.expected_sum("copy");
    let beta = u32::from(s.beta);
    let giv = (go + 2 * gc) as f64;
    let swaps = if p.swap { 4.0 * (s.n + 1) as f64 } else { 0.0 };
    let c = to
        + 2.0 * tc
        + giv * f64::from(2 * (beta - 2))
        + f64::from(map.uniform_bits() - 2)
        + f64::from(map.inner_width() - 2)
        + swaps;
    // Each gated block is n - 1 Toffolis with probability 1/2: variance (n - 1)^2 / 4.
    let var = if p.tw.gated {
        let (mo, mi) = (f64::from(p.outer.1), f64::from(p.inner.1));
        let inner = if p.tw.keep_ladder { 0.0 } else { 2.0 };
        let outer = if p.tw.outer_ladder { 0.0 } else { 1.0 };
        (outer * (mo - 1.0).powi(2) + inner * (mi - 1.0).powi(2)) / 4.0
    } else {
        0.0
    };
    (c, var)
}

fn passes(s: &SaSpec, p: Params, samples: usize) -> Evaluation {
    let (lm, ops, led) = build(s, p);
    let ev = eval(s, p, &lm, &ops, samples).unwrap_or_else(|e| panic!("{:?}: {e}", p.tw));
    assert_eq!(ev.rounding_error, Exact::zero());
    assert!(ev.facts.nested_validated);
    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
        assert_eq!(v.status, AxisStatus::Verified, "{axis}");
    }
    let (want, var) = ledger_c_step(&led, s, p);
    let tol = if var > 0.0 {
        6.0 * (var / samples as f64).sqrt()
    } else {
        1e-9
    };
    assert!(
        (ev.toffoli - want).abs() <= tol,
        "{:?}: harness {} vs expected ledger {want} (tolerance {tol})",
        p.tw,
        ev.toffoli
    );
    ev
}

#[test]
fn c1_levers_pass_on_exact_specs() {
    for (s, name) in [(spec3(), "N=3"), (spec5(), "N=5")] {
        for tw in TWEAKS {
            for swap in [true, false] {
                let p = Params {
                    swap,
                    tw: Tweaks::parse(tw),
                    ..P
                };
                let ev = passes(&s, p, 1 << 14);
                println!(
                    "{name} {tw} swap {swap}: C_step {} Q_peak {}",
                    ev.toffoli, ev.qubits
                );
            }
        }
    }
}

/// `g` saves exactly half the recompute on average, and `u` changes no Toffoli and no Givens.
#[test]
fn c1_levers_cost_what_they_claim() {
    let s = spec5();
    let led = |tw: &str| {
        build(
            &s,
            Params {
                tw: Tweaks::parse(tw),
                ..P
            },
        )
        .2
    };
    let sum = |l: &super::ledger::Ledger| {
        let (o, _) = l.expected_sum("outer");
        let (c, _) = l.expected_sum("copy");
        o + 2.0 * c
    };
    let base = led("imchx");
    let g = led("imchxg");
    let (mo, mi) = (f64::from(P.outer.1), f64::from(P.inner.1));
    let want = (mo - 1.0 + 2.0 * (mi - 1.0)) / 2.0;
    assert!((sum(&base) - sum(&g) - want).abs() < 1e-9);
    let o = led("imchxu");
    assert!((sum(&base) - sum(&o)).abs() < 1e-9);
    assert_eq!(base.sum("copy").1, o.sum("copy").1, "same Givens");
    // In range: the write (with the compact layout's k_i ANDs for a one-body b) costs at most
    // E - 1 + k_i Toffolis, and less than the ragged write.
    let r = led("imchxr");
    let e = (s.r * s.b + s.n) as f64;
    let write = |l: &super::ledger::Ledger| l.expected_sum("copy: RPREP angle read").0;
    let k_i = f64::from(P.inner.0);
    println!("write: ragged {} in range {} (E {e})", write(&o), write(&r));
    assert!(write(&r) <= e - 1.0 + k_i, "in-range write {}", write(&r));
    assert!(write(&r) < write(&o));
}

#[test]
fn c1_levers_run_on_a_padded_spec() {
    let s = spec4();
    for tw in TWEAKS {
        for swap in [false, true] {
            let p = Params {
                outer: (3, 20),
                inner: (2, 20),
                outer_a: 2,
                swap,
                tw: Tweaks::parse(tw),
                ..P
            };
            let (lm, ops, _) = build(&s, p);
            match eval(&s, p, &lm, &ops, 1 << 13) {
                Ok(ev) => println!("N=4 {tw} swap {swap}: C {} Q {}", ev.toffoli, ev.qubits),
                Err(e) => assert!(e.contains("rounding error"), "{tw} {swap}: {e}"),
            }
        }
    }
}

/// Mutants of the gated erasure (in the walk): the conditioned phase `(-1)^lt` dropped, and the
/// correction gated on the wrong outcome.
#[test]
fn gated_erasure_mutants_are_rejected() {
    let s = spec5();
    for tw in ["g", "imchxg", "imchxgu"] {
        let p = Params {
            tw: Tweaks::parse(tw),
            ..P
        };
        let (lm, ops, _) = build(&s, p);
        assert!(ops.iter().any(|o| o.kind == K::PushCondition));
        // Drop the unconditioned CZ / Z inside condition blocks: the gated (-1)^lt.
        let mut depth = 0;
        let no_phase: Vec<Op> = ops
            .iter()
            .filter(|o| {
                match o.kind {
                    K::PushCondition => depth += 1,
                    K::PopCondition => depth -= 1,
                    _ => {}
                }
                !(depth > 0 && matches!(o.kind, K::CZ | K::Z) && o.c_condition == NONE)
            })
            .copied()
            .collect();
        assert!(no_phase.len() < ops.len());
        // Flip the outcome around every condition block.
        let mut flipped = Vec::with_capacity(ops.len() + 64);
        let mut open: Vec<u32> = Vec::new();
        for o in &ops {
            let inv = |c: u32| {
                let mut x = Op::new(K::BitInvert);
                x.c_target = c;
                x
            };
            match o.kind {
                K::PushCondition => {
                    flipped.push(inv(o.c_condition));
                    flipped.push(*o);
                    open.push(o.c_condition);
                }
                K::PopCondition => {
                    flipped.push(*o);
                    flipped.push(inv(open.pop().unwrap()));
                }
                _ => flipped.push(*o),
            }
        }
        for (name, m) in [("no phase", no_phase), ("flipped outcome", flipped)] {
            let e = eval(&s, p, &lm, &m, 1 << 12);
            assert!(e.is_err(), "{tw}: {name} mutant must be rejected");
            println!("{tw} {name}: {}", e.err().unwrap());
        }
    }
}

/// Mutants of lever `d`: the controlled fixup's control dropped (applied everywhere), or
/// skipped.
#[test]
fn drop_alt_mutants_are_rejected() {
    let s = spec5();
    for tw in ["md", "imchxgrd"] {
        let p = Params {
            tw: Tweaks::parse(tw),
            ..P
        };
        let (lm, ops, _) = build(&s, p);
        let good = eval(&s, p, &lm, &ops, 1 << 12);
        assert!(good.is_ok(), "{tw}: {:?}", good.err());
        // The controlled fixup is the only place a one-hot is written under a control that is
        // not an index bit; without the lever the stream has no BitStore0 before the outer
        // UNPREPARE's first fixup. Drop every CZ conditioned on the controlled fixup's parity
        // bit (the first BitStore0 after the SELECT segment).
        let sel = ops
            .iter()
            .rposition(|o| o.kind == K::Segment && o.r_target == crate::circuit::SEG_UNPREPARE)
            .unwrap();
        let p_bit = ops[sel..]
            .iter()
            .find(|o| o.kind == K::BitStore0)
            .unwrap()
            .c_target;
        let skipped: Vec<Op> = ops
            .iter()
            .enumerate()
            .filter(|(i, o)| {
                !(*i > sel && matches!(o.kind, K::CZ | K::Z) && o.c_condition == p_bit)
            })
            .map(|(_, o)| *o)
            .collect();
        assert!(skipped.len() < ops.len());
        let e = eval(&s, p, &lm, &skipped, 1 << 12);
        assert!(
            e.is_err(),
            "{tw}: skipped controlled fixup must be rejected"
        );
        println!("{tw} skipped controlled fixup: {}", e.err().unwrap());
    }
}

/// Mutants of the one-hot RPREP: its erasure's fixup dropped or shifted by one leaf, the angle
/// register not unloaded, a fan-out transition skipped; for the split one-hot (`v`, `w`) also the
/// group corrections skipped and the group bits not written.
#[test]
fn onehot_mutants_are_rejected() {
    let s = spec5();
    for tw in [
        "u", "imchxu", "imchxgu", "imchxgr", "v", "imchxgrv", "imchxgrw",
    ] {
        let p = Params {
            tw: Tweaks::parse(tw),
            ..P
        };
        let last = if p.tw.hot_groups > 0 { 6 } else { 4 };
        for fault in 1..=last {
            onehot::FAULT.with(|c| c.set(fault));
            let (lm, ops, _) = build(&s, p);
            onehot::FAULT.with(|c| c.set(0));
            let e = eval(&s, p, &lm, &ops, 1 << 12);
            assert!(e.is_err(), "{tw}: one-hot fault {fault} must be rejected");
            println!("{tw} fault {fault}: {}", e.err().unwrap());
        }
    }
}

/// `g`, `u` off: the `sa-toff` bundles emit the same ops as before these levers existed (the
/// pinned digests in `tests/sa_digests.rs` check the real circuits; this checks the parse).
#[test]
fn old_bundles_are_unchanged() {
    for tw in ["all", "imchx", "imchxL", "none", "i", "l", "L"] {
        let t = Tweaks::parse(tw);
        assert!(!t.gated && !t.onehot, "{tw}");
    }
    assert!(Tweaks::parse("imchxgu").gated && Tweaks::parse("imchxgu").onehot);
    for tw in ["all", "imchx", "imchxgr", "imchxlgrd", "none"] {
        assert_eq!(Tweaks::parse(tw).hot_groups, 0, "{tw}");
    }
    assert_eq!(Tweaks::parse("imchxgrv").hot_groups, 2);
    assert_eq!(Tweaks::parse("imchxgrw").hot_groups, 3);
    assert!(Tweaks::parse("v").onehot && Tweaks::parse("w").onehot);
    assert!(!Tweaks::parse("g").any_toff());
}

fn pinned(id: &str) -> Box<dyn EncodingSpec> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::spec::load(root, id).unwrap()
}

/// Static expected `C_step` (the expected-count ledger) and static `Q_peak` (the harness's
/// static pass) of every bundle in `SA_TWEAKS` (comma separated; default below) on the specs in
/// `SA_SPECS`, with `FEMOCO_SA_INNER_A` overridable at run time by `SA_INNER_A`. No lanes.
#[test]
#[ignore = "pinned specs; run in release"]
fn c1_scan() {
    let tws = std::env::var("SA_TWEAKS")
        .unwrap_or_else(|_| "imchx,imchxL,imchxg,imchxgu,imchxgr,imchxlgr".into());
    let ids = std::env::var("SA_SPECS")
        .unwrap_or_else(|_| "reiher-sa-v1,li-sa-v1,reiher-sa-gb-r15-v1,li-sa-gb-r14-v1".into());
    let inner_as: Vec<Option<usize>> = std::env::var("SA_INNER_A").map_or(vec![None], |v| {
        v.split(',').map(|x| x.parse().ok()).collect()
    });
    for id in ids.split(',') {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for tw in tws.split(',') {
            for &ia in &inner_as {
                let base = Params::for_spec(s);
                let p = Params {
                    tw: Tweaks::parse(tw),
                    inner_a: ia.unwrap_or(base.inner_a),
                    ..base
                };
                let map = lane_map(s, p).unwrap();
                let mut b = Builder::new(2 * s.n);
                b.declare_uniform(map.uniform_bits());
                let led = emit(s, &map, &mut b, p);
                let ops = b.finish();
                let file = ops_file(&ops);
                let layout = crate::sim::Layout {
                    system: s.system_qubits(),
                    uniform: map.uniform_bits(),
                };
                let inner = map
                    .inner_bits()
                    .map(|(lo, width)| crate::sim::Inner { lo, width });
                let tr = givens_tracker(s);
                let c = crate::sim::compile_sa(&file.ops, &layout, tr, inner).unwrap();
                let q = c.q_peak + tr.map_or(0, |t| t.phase_gradient_qubits());
                let (cs, _) = ledger_c_step(&led, s, p);
                println!(
                    "{id} {tw} inner_a {}: C_step {cs:.1} Q_peak {q} product {:.4e} ops {} sha {}",
                    p.inner_a,
                    cs * q as f64,
                    ops.len(),
                    &hex::encode(file.sha256)[..16]
                );
                if std::env::var("SA_ROWS").is_ok() {
                    for (r, e) in led.rows.iter().zip(&led.expected) {
                        println!("{:>10.1} {:>5}  {}", e, r.2, r.0);
                    }
                }
            }
        }
    }
}

/// K lanes of the harness's `evaluate` on pinned specs: `SA_SPECS`, `SA_TWEAKS` (comma
/// separated), `SA_INNER_A` (default the spec's), `SA_K` (default 64). Prints the harness's
/// `C_step` and `Q_peak` next to the ledger's expected `C_step`. Heavy on Li; run in release.
#[test]
#[ignore = "pinned specs; run in release"]
fn c1_lanes_pinned() {
    let ids = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1".into());
    let tws = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrvd".into());
    let k: usize = std::env::var("SA_K").map_or(64, |v| v.parse().unwrap());
    let ia: Option<usize> = std::env::var("SA_INNER_A")
        .ok()
        .and_then(|v| v.parse().ok());
    let oa: Option<usize> = std::env::var("SA_OUTER_A")
        .ok()
        .and_then(|v| v.parse().ok());
    let mu: Option<(u32, u32)> = std::env::var("SA_MU").ok().map(|v| {
        let (o, i) = v.split_once(',').expect("SA_MU=o,i");
        (o.parse().unwrap(), i.parse().unwrap())
    });
    for id in ids.split(',') {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for tw in tws.split(',') {
            let base = Params::for_spec(s);
            let p = Params {
                tw: Tweaks::parse(tw),
                inner_a: ia.unwrap_or(base.inner_a),
                outer_a: oa.unwrap_or(base.outer_a),
                outer: mu.map_or(base.outer, |m| (base.outer.0, m.0)),
                inner: mu.map_or(base.inner, |m| (base.inner.0, m.1)),
                ..base
            };
            let (lm, ops, led) = build(s, p);
            let (want, _) = ledger_c_step(&led, s, p);
            match eval(s, p, &lm, &ops, k) {
                Ok(ev) => println!(
                    "LANES {id} {tw} inner_a {} K {k}: C_step {} Q_peak {} (ledger expected {want:.1}) rounding {:.4e}",
                    p.inner_a,
                    ev.toffoli,
                    ev.qubits,
                    ev.rounding_error.to_f64()
                ),
                Err(e) => panic!("{id} {tw}: {e}"),
            }
        }
    }
}

/// Lever `.d` changes only the Reiher inner alias tables. Sample the complete circuit through
/// the trusted evaluator and check that moving two lanes out of the legal rounding interval is
/// rejected before any circuit result can be accepted.
#[test]
#[ignore = "pinned Reiher spec; run in release"]
fn sparse_reiher_exact_and_mutant() {
    let boxed = pinned("reiher-sa-est-v1");
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let base = Params::for_spec(s);
    let p = Params {
        tw: Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs+Rq17_1.14.e.d"),
        outer: (base.outer.0, 8),
        inner: (base.inner.0, 8),
        inner_a: 2,
        outer_a: 2,
        ..base
    };
    let (lanemap, ops, _) = build(s, p);
    let ev = eval(s, p, &lanemap, &ops, 128).unwrap();
    println!("sparse Reiher: C {} Q {}", ev.toffoli, ev.qubits);

    let mut map = lane_map(s, p).unwrap();
    let t = &mut map.inner[s.n];
    let i = (0..28)
        .find(|&i| t.keep[i] <= 253 && t.alt[i] as usize != i)
        .unwrap();
    t.keep[i] += 2;
    let err = eval(s, p, &map.to_bytes(), &ops, 128).unwrap_err();
    assert!(err.contains("floor or the ceiling"), "{err}");
}

/// The outcome-gated keep release of `narrow.rs` in `sa-toff` (levers `k`
/// outer, `j` inner) and the split one-hot with any group count (a digit `G`), alone and
/// combined with every lever they touch.
const RECOMBINE: [&str; 20] = [
    "iy",
    "imchxgry",
    "imchxgrdky4",
    "imchxgrdkjy3",
    "dk",
    "imchxdk",
    "imchxgrdk",
    "mj",
    "imchxj",
    "imchxgrj",
    "imchxgrdkj",
    "imchgrdkj",
    "imchxgurdkj",
    "imchxhgrdkj",
    "4",
    "imchxgrd4",
    "imchxgrdk3",
    "imchxgrdkj4",
    "imchxgrdj5",
    "imchxgrdkj6",
];

/// spec4's parameters at 20-bit keeps (as `c1_levers_run_on_a_padded_spec`): the rounding rule
/// passes and the inner tables have nonzero keeps.
const WIDE: Params = Params {
    outer: (3, 20),
    inner: (2, 20),
    outer_a: 2,
    ..P
};

/// An upper bound on the variance of one lane's Toffoli count: every Toffoli inside a condition
/// block is gated on a fair outcome, so the variance is at most `(sum of their counts)^2 / 4`.
fn gated_var_bound(ops: &[Op]) -> f64 {
    let (mut depth, mut gated) = (0usize, 0usize);
    for op in ops {
        match op.kind {
            K::PushCondition => depth += 1,
            K::PopCondition => depth -= 1,
            K::CCX | K::CCZ if depth > 0 || op.c_condition != NONE => gated += 1,
            _ => {}
        }
    }
    (gated as f64).powi(2) / 4.0
}

/// Every such bundle passes `evaluate` end to end on the exact N = 3 and N = 5 specs, with
/// both spin conventions and several block counts for the reads and re-reads, every axis
/// verified, and the harness's `C_step` equal to the expected-count ledger within sampling noise.
#[test]
fn recombined_levers_pass_on_exact_specs() {
    let samples = 1 << 13;
    // spec4 at 20-bit keeps passes the rounding rule, and unlike the dyadic N = 3 / N = 5 specs
    // its inner tables have nonzero keeps, so the inner re-read (`j`) has work to do.
    for (s, name, wide) in [
        (spec3(), "N=3", false),
        (spec5(), "N=5", false),
        (spec4(), "N=4 wide", true),
    ] {
        for tw in RECOMBINE {
            for (oa, ia) in [(1, 1), (0, 2), (2, 0)] {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue; // lever y needs the SpinSwap SELECT
                    }
                    let base = if wide { WIDE } else { P };
                    let p = Params {
                        swap,
                        outer_a: oa,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia} {swap}: {e}"));
                    if !wide {
                        assert_eq!(ev.rounding_error, Exact::zero());
                    }
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw} oa {oa} ia {ia}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                }
            }
        }
        println!("{name}: {} recombined bundles pass", RECOMBINE.len());
    }
}

/// Mutants of the carried-over keep release: the gated re-read's comparison phase dropped
/// (faults 7 / 9) and its outcomes left out of the final fixup (8 / 10), outer and inner. Each
/// must be rejected by the harness.
#[test]
fn keep_release_mutants_are_rejected() {
    // spec4 at 20-bit keeps: both keep tables have nonzero entries, so both re-reads matter (on
    // the dyadic N = 5 spec every inner keep is 0 and the inner mutants would be vacuous).
    let s = spec4();
    for (tw, faults) in [
        ("imchxdk", [7u8, 8]),
        ("imchxgrdk", [7, 8]),
        ("imchxj", [9, 10]),
        ("imchxgrdkj4", [9, 10]),
        ("imchgrdkj", [9, 10]),
    ] {
        for fault in faults {
            let caught = [(1usize, 1usize), (2, 2)].iter().any(|&(oa, ia)| {
                let p = Params {
                    outer_a: oa,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..WIDE
                };
                onehot::FAULT.with(|c| c.set(fault));
                let (lm, ops, _) = build(&s, p);
                onehot::FAULT.with(|c| c.set(0));
                eval(&s, p, &lm, &ops, 1 << 12).is_err()
            });
            assert!(caught, "{tw}: fault {fault} must be rejected");
        }
    }
    // Lever y (the Majorana cut) combined with k and j: id's gated recompute
    // dropped (11) and the classical pass never toggled (12).
    for tw in ["imchxgry", "imchxgrdkjy4"] {
        for fault in [11u8, 12] {
            let p = Params {
                tw: Tweaks::parse(tw),
                ..WIDE
            };
            onehot::FAULT.with(|c| c.set(fault));
            let (lm, ops, _) = build(&s, p);
            onehot::FAULT.with(|c| c.set(0));
            assert!(
                eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                "{tw}: fault {fault} must be rejected"
            );
        }
    }
    // The split one-hot's own mutants with a general group count.
    for tw in ["imchxgrd4", "imchxgrdkj5"] {
        let p = Params {
            tw: Tweaks::parse(tw),
            ..WIDE
        };
        for fault in [1u8, 2, 5, 6] {
            onehot::FAULT.with(|c| c.set(fault));
            let (lm, ops, _) = build(&s, p);
            onehot::FAULT.with(|c| c.set(0));
            assert!(
                eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                "{tw}: one-hot fault {fault} must be rejected"
            );
        }
    }
}

/// The letters parse as documented and leave every bundle without them as it was.
#[test]
fn recombine_letters_parse() {
    let t = Tweaks::parse("imchxgrdkj4");
    assert!(t.keep_release && t.inner_release && t.onehot && t.drop_alt);
    assert_eq!(t.hot_groups, 4);
    assert_eq!(Tweaks::parse("imchxgrv").hot_groups, 2);
    assert_eq!(Tweaks::parse("7").hot_groups, 7);
    assert!(Tweaks::parse("7").onehot);
    for tw in [
        "all",
        "imchx",
        "imchxL",
        "imchxgrvd",
        "imchxgrwd",
        "imchxlgrp",
        "none",
    ] {
        let t = Tweaks::parse(tw);
        assert!(!t.keep_release && !t.inner_release, "{tw}");
    }
    assert_eq!(Tweaks::parse("all").hot_groups, 0);
}

/// Lever `z`: paired group corrections of the split one-hot. Every bundle
/// passes `evaluate` end to end on the exact N = 3 / N = 5 specs and the wide N = 4 spec (both
/// SELECT variants where the bundle allows), with verified axes and the harness's `C_step`
/// equal to the ledger; pairing never costs more than the unpaired split at the same `G`.
const PAIRED: [&str; 8] = [
    "3z",
    "imchxgr3z",
    "imchxgrd3z",
    "imchxgrdk3z",
    "imchxgrdky3z",
    "imchxgrd4z",
    "imchxgrdky5z",
    "imchxgrdkj7z",
];

#[test]
fn paired_groups_pass_on_exact_specs() {
    let samples = 1 << 13;
    for (s, name, wide) in [
        (spec3(), "N=3", false),
        (spec5(), "N=5", false),
        (spec4(), "N=4 wide", true),
    ] {
        for tw in PAIRED {
            for (oa, ia) in [(1, 1), (0, 2), (2, 0)] {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue;
                    }
                    let base = if wide { WIDE } else { P };
                    let p = Params {
                        swap,
                        outer_a: oa,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia} {swap}: {e}"));
                    if !wide {
                        assert_eq!(ev.rounding_error, Exact::zero());
                    }
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                    // The same bundle without `z` costs at least as much.
                    let plain = Params {
                        tw: Tweaks::parse(&tw.replace('z', "")),
                        ..p
                    };
                    let (_, _, led0) = build(&s, plain);
                    assert!(led.sum("copy").0 <= led0.sum("copy").0, "{name} {tw}");
                }
            }
        }
        println!("{name}: {} paired bundles pass", PAIRED.len());
    }
}

/// Lever `z`'s mutants: the product's `X Y` fan-out skipped (20) and the two parities swapped
/// between the group bits (21), plus the split one-hot's own faults (corrections skipped, group
/// bits not written). Each must be rejected by the harness.
#[test]
fn paired_mutants_are_rejected() {
    for (s, base) in [(spec5(), P), (spec4(), WIDE)] {
        for tw in ["imchxgr3z", "imchxgrdk5z"] {
            for fault in [20u8, 21, 5, 6] {
                let p = Params {
                    tw: Tweaks::parse(tw),
                    ..base
                };
                onehot::FAULT.with(|c| c.set(fault));
                let (lm, ops, _) = build(&s, p);
                onehot::FAULT.with(|c| c.set(0));
                let e = eval(&s, p, &lm, &ops, 1 << 12);
                assert!(e.is_err(), "{tw}: fault {fault} must be rejected");
            }
        }
    }
}

/// An N = 4 spec with six square rows (`R = 6, B = 3, C = 1`): with `G = 2` or `3` the folded
/// layout (lever `f`) has whole-row classes, and the padded one-body rows are placed one by one.
fn spec6() -> SaSpec {
    let bytes = payload(
        4,
        (6, 3, 1),
        &[1.0, -2.0, 1.0, 4.0],
        &[1.0, -4.0, 2.0, 1.0, -1.0, 3.0],
        &[
            1.0, -1.0, 2.0, 1.0, 2.0, -1.0, -1.0, 1.0, 1.0, 2.0, 1.0, -2.0, 1.0, 1.0, -1.0, 4.0,
            -1.0, 2.0,
        ],
    );
    parse_payload("test-sa6-v1", &bytes).unwrap()
}

/// Lever `f` (folded split one-hot): every bundle passes end to end on the six-row spec (two
/// classes at `G = 3`, three at `G = 2`), the wide N = 4 spec (one class at `G = 2`) and the
/// N = 5 spec (no class: every leaf placed one by one), with verified axes and the ledger equal to
/// the harness; folding never costs more Toffolis than the contiguous split at the same `G`.
const FOLDED: [&str; 12] = [
    "imchxgrdkyb",
    "imchxgrdky3zfab",
    "imchxgrdky5zab",
    "imchxgrda",
    "imchxgrd3za",
    "imchxgrdky3zfa",
    "imchxgrd2f",
    "imchxgrd3zf",
    "imchxgrdk3zf",
    "imchxgrdky3zf",
    "imchxgrdkj3zf",
    "imchxgrdky5zf",
];

#[test]
fn folded_groups_pass_on_exact_specs() {
    let samples = 1 << 13;
    for (s, name, base) in [
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
    ] {
        for tw in FOLDED {
            for (oa, ia) in [(1, 1), (0, 2), (2, 0)] {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue;
                    }
                    let p = Params {
                        swap,
                        outer_a: oa,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia} {swap}: {e}"));
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                    let plain = Params {
                        tw: Tweaks::parse(&tw.replace('f', "")),
                        ..p
                    };
                    let (_, _, led0) = build(&s, plain);
                    println!(
                        "{name} {tw} oa {oa} ia {ia} {swap}: copy {} vs unfolded {}",
                        led.sum("copy").0,
                        led0.sum("copy").0
                    );
                }
            }
        }
    }
}

/// Lever `f`'s mutants: the class flags left out of the erasure fixup (22), class 0's slots
/// shifted (23), and the group bits not written (6). Each must be rejected.
#[test]
fn folded_mutants_are_rejected() {
    let s = spec6();
    for (tw, faults) in [
        ("imchxgrd2f", vec![22u8, 23, 6]),
        ("imchxgrd3zf", vec![22, 23, 6]),
        ("imchxgrd3zfa", vec![24]),
        ("imchxgrda", vec![24]),
        ("imchxgrdky3zfab", vec![25, 26]),
        ("imchxgrdkyb", vec![25, 26]),
    ] {
        for fault in faults {
            let p = Params {
                tw: Tweaks::parse(tw),
                ..WIDE6
            };
            onehot::FAULT.with(|c| c.set(fault));
            let (lm, ops, _) = build(&s, p);
            onehot::FAULT.with(|c| c.set(0));
            assert!(
                eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                "{tw}: fault {fault} must be rejected"
            );
        }
    }
}

/// [`WIDE`] with a 4-bit outer index (the six-row spec has 10 outer items).
const WIDE6: Params = Params {
    outer: (4, 20),
    ..WIDE
};

/// Lever `E` (angle register embedded in the unsplit one-hot), alone and
/// composed with `k`, `y`, the ladder and the index checkpoint `b`: every bundle
/// passes `evaluate` end to end on the exact N = 3 / N = 5 specs and the wide N = 4 spec, with
/// verified axes and the harness's `C_step` equal to the ledger.
const RECOMBINE2: [&str; 8] = [
    "imchxgrE",
    "imchxgrdkE",
    "imchxgrdkEy",
    "imchxlgrdkE",
    "imchxgrdkEyb",
    "imchxgrdkyb3zfa",
    "imchxgrdkyb3zf",
    "imchxgrdkyb5zfa",
];

#[test]
fn embed_passes_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, wide) in [
        (spec3(), "N=3", false),
        (spec5(), "N=5", false),
        (spec4(), "N=4 wide", true),
    ] {
        for tw in RECOMBINE2 {
            for (oa, ia) in [(1, 1), (0, 2), (2, 0)] {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue;
                    }
                    let base = if wide { WIDE } else { P };
                    let p = Params {
                        swap,
                        outer_a: oa,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia} {swap}: {e}"));
                    if !wide {
                        assert_eq!(ev.rounding_error, Exact::zero());
                    }
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw} oa {oa} ia {ia}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                }
            }
        }
        println!("{name}: {} embed bundles pass", RECOMBINE2.len());
    }
}

/// Lever `E`'s mutants, each rejected by the harness: 14 the embedded register missing the
/// non-pivot leaves, 19 one Gauss-Jordan CNOT dropped (and the one-hot faults 1, 3 with `E`).
#[test]
fn embed_mutants_are_rejected() {
    for (tw, faults) in [
        ("imchxgrdkE", vec![1u8, 3, 14, 19]),
        ("imchxgrdkEyb", vec![14, 19]),
    ] {
        for fault in faults {
            let caught = [spec3(), spec5(), spec4()]
                .into_iter()
                .enumerate()
                .any(|(i, s)| {
                    let base = if i == 2 { WIDE } else { P };
                    [(1usize, 1usize), (2, 0)].iter().any(|&(oa, ia)| {
                        let p = Params {
                            outer_a: oa,
                            inner_a: ia,
                            tw: Tweaks::parse(tw),
                            ..base
                        };
                        onehot::FAULT.with(|c| c.set(fault));
                        let (lm, ops, _) = build(&s, p);
                        onehot::FAULT.with(|c| c.set(0));
                        eval(&s, p, &lm, &ops, 1 << 12).is_err()
                    })
                });
            assert!(caught, "{tw}: fault {fault} must be rejected");
        }
    }
}

/// `E` parses as documented and no bundle without the letter turns it on.
#[test]
fn embed_letter_parses() {
    let t = Tweaks::parse("imchxgrdkEy");
    assert!(t.embed && t.keep_release && t.majorana_cut);
    for tw in [
        "all",
        "none",
        "imchx",
        "imchxL",
        "imchxgrvd",
        "imchxlgrp",
        "imchxgrdkjy4",
        "3z",
    ] {
        assert!(!Tweaks::parse(tw).embed, "{tw}");
    }
}

/// Levers `S` (signs as a lane phase at PREPARE), `R` (inner keep test
/// released and recomputed), `N` (spin select released across `V^dagger M V`), `O` (`pos_e` from
/// the one-hot) and `B` (the checkpoint cleared by measurement), alone and together, on the
/// unsplit, split, paired and folded one-hot. Every bundle passes `evaluate` end to end on the
/// exact N = 3 / N = 5 specs, the wide N = 4 spec (nonzero inner keeps, so `R` and `S`'s
/// `lt`-dependent half have work to do) and the six-row spec (folded classes), with every
/// axis verified and the harness's `C_step` equal to the ledger.
const MEASURED: [&str; 19] = [
    "imchxgrdky5zfabHSRNOBWQ",
    "imchxgrdky3zfabSRNOBWQ",
    "imchxgrdky3zfabSRNOBW",
    "imchxgrdky5zfabHSRNOBW",
    "imchxgrdkyW",
    "imchxgrdky5zfabHSRNOB",
    "imchxgrdky3zfabHKSRNOB",
    "imchxgrdky3zfabS",
    "imchxgrdky3zfabR",
    "imchxgrdky3zfabN",
    "imchxgrdky3zfabO",
    "imchxgrdky3zfabB",
    "imchxgrdky3zfabSRNOB",
    "imchxgrdkybSRNOB",
    "imchxgrdkySRNO",
    "imchxgrdky3zabSRNOB",
    "imchxgrdky5zfabSRNOB",
    "imchxgrdky2fabSRNOB",
    "imchxgrdkEybSRNOB",
];

#[test]
fn measured_unload_levers_pass_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base, exact) in [
        (spec3(), "N=3", P, true),
        (spec5(), "N=5", P, true),
        (spec4(), "N=4 wide", WIDE, false),
        (spec6(), "R=6", WIDE6, false),
    ] {
        for tw in MEASURED {
            for (oa, ia) in [(1, 1), (0, 2), (2, 0)] {
                let p = Params {
                    swap: true,
                    outer_a: oa,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples)
                    .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia}: {e}"));
                if exact {
                    assert_eq!(ev.rounding_error, Exact::zero());
                }
                assert!(ev.facts.nested_validated);
                for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                    let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                    assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                }
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} oa {oa} ia {ia}: harness {} vs ledger {want} (tol {tol})",
                    ev.toffoli
                );
            }
        }
        println!("{name}: {} measured-unload bundles pass", MEASURED.len());
    }
}

/// What each lever costs per copy, against the same bundle without it (static ledger, exact):
/// `S` one Toffoli (two ANDs, one swapped bit fewer), `R` one comparison, `N` one spin select,
/// `O` at most one Toffoli per group pair (the fan-out of `pos_e`), `B` never more.
#[test]
fn measured_unload_levers_cost_what_they_claim() {
    let s = spec6();
    let base = "imchxgrdky3zfab";
    let copy = |tw: &str| {
        let p = Params {
            tw: Tweaks::parse(tw),
            ..WIDE6
        };
        build(&s, p).2.sum("copy").0
    };
    let c0 = copy(base);
    let mu = u64::from(WIDE6.inner.1);
    assert_eq!(copy(&format!("{base}S")), c0 + 1, "S");
    let r = copy(&format!("{base}R"));
    assert!(r > c0 && r <= c0 + mu, "R: {r} vs {c0}");
    assert_eq!(copy(&format!("{base}N")), c0 + 1, "N");
    assert!(copy(&format!("{base}O")) <= c0 + 1, "O");
    assert!(copy(&format!("{base}B")) <= c0, "B");
}

/// The new levers' mutants: `S`'s `CZ(t, s_alt)` (40) or `CZ(t lt, s_own ^ s_alt)` (41) skipped,
/// `B`'s identity fix skipped before the measurement (42), `O`'s fan-out of `pos_e` skipped (43),
/// `R`'s `Z` on the recomputed keep test skipped (44). Each must be rejected by the harness on a
/// spec where it matters (the wide specs have nonzero inner keeps, so `lt` takes both values).
#[test]
fn measured_unload_mutants_are_rejected() {
    for (s, base, name) in [(spec4(), WIDE, "N=4 wide"), (spec6(), WIDE6, "R=6")] {
        for (tw, faults) in [
            ("imchxgrdky3zfabS", vec![40u8, 41]),
            ("imchxgrdkySRNO", vec![40, 41, 43, 44]),
            ("imchxgrdky3zfabSRNOB", vec![40, 41, 42, 43, 44]),
            ("imchxgrdkybB", vec![42]),
            ("imchxgrdky5zfabHSRNOB", vec![40, 41, 42, 43, 44]),
            ("imchxgrdky3zfabSRNOBW", vec![45]),
            ("imchxgrdkyW", vec![45]),
            ("imchxgrdky3zfabSRNOBWQ", vec![45]),
        ] {
            for fault in faults {
                let p = Params {
                    tw: Tweaks::parse(tw),
                    ..base
                };
                onehot::FAULT.with(|c| c.set(fault));
                let (lm, ops, _) = build(&s, p);
                onehot::FAULT.with(|c| c.set(0));
                assert!(
                    eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                    "{name} {tw}: fault {fault} must be rejected"
                );
            }
        }
    }
}

#[test]
fn measured_unload_letters_parse() {
    let t = Tweaks::parse("imchxgrdky3zfabSRNOB");
    assert!(t.sign_phase && t.lt_release && t.spin_unload && t.pos_from_hot && t.measured_ck);
    assert!(t.paired_groups && t.fold && t.measured_unload && t.checkpoint);
    let a = Tweaks::parse("all");
    assert!(!a.sign_phase && !a.lt_release && !a.spin_unload && !a.pos_from_hot && !a.measured_ck);
    // A bundle without these letters parses as before.
    let old = Tweaks::parse("imchxgrdky3zfab");
    assert!(!old.sign_phase && !old.lt_release && !old.spin_unload);
    assert!(!old.pos_from_hot && !old.measured_ck);
}

/// Lever `H` (item one-hot inner read, `itemhot.rs`): every bundle passes end to end on the six-row
/// spec (6 items, 3 slots), the wide N = 4 spec and the exact N = 3 / N = 5 specs, both SELECTs
/// where allowed, with verified axes and the ledger equal to the harness.
const ITEMHOT: [&str; 8] = [
    "imchxHK",
    "imchxgrdky3zfabHK",
    "imchxH",
    "imchxgrH",
    "imchxgrdkH",
    "imchxgrdky3zfabH",
    "imchxgrdky5zfabH",
    "imchxgrdkyabH",
];

#[test]
fn item_hot_passes_on_exact_specs() {
    let samples = 1 << 13;
    for (s, name, base) in [
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
        (spec3(), "N=3", P),
    ] {
        for tw in ITEMHOT {
            for (oa, ia) in [(1, 1), (0, 2)] {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue;
                    }
                    let p = Params {
                        swap,
                        outer_a: oa,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia} {swap}: {e}"));
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                }
            }
        }
    }
}

/// Lever `H`'s mutants: the group bit not written (30), the one-hot's erasure fixup skipped (31),
/// the read's `X Y` cancellation skipped (32), the erasure's `X Y` phases skipped (33). Each must
/// be rejected by the harness.
#[test]
fn item_hot_mutants_are_rejected() {
    // A fault must be rejected on at least one of the specs (on the dyadic six-row spec several
    // tables are trivial, so a cancellation term can be identically zero there).
    for (tw, faults) in [
        ("imchxgrH", vec![30u8, 31, 32, 33]),
        ("imchxgrdky3zfabH", vec![30, 31, 32, 33]),
        ("imchxgrHK", vec![34, 30]),
    ] {
        for fault in faults {
            let caught = [(spec6(), WIDE6), (spec4(), WIDE)]
                .into_iter()
                .any(|(s, base)| {
                    let p = Params {
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    onehot::FAULT.with(|c| c.set(fault));
                    let (lm, ops, _) = build(&s, p);
                    onehot::FAULT.with(|c| c.set(0));
                    eval(&s, p, &lm, &ops, 1 << 12).is_err()
                });
            assert!(caught, "{tw}: item-hot fault {fault} must be rejected");
        }
    }
}

// ---- exclusivity levers ----------------------------------------------------------------------

/// Runs every bundle of `tws` end to end on the six-row, wide N = 4 and exact N = 3 / N = 5 specs
/// (both SELECTs where allowed, two block-count pairs): verified axes, the ledger equal to the
/// harness within the gated-sampling tolerance.
fn excl_bundles_pass(tws: &[&str]) {
    let samples = 1 << 13;
    for (s, name, base) in [
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
        (spec3(), "N=3", P),
    ] {
        for &tw in tws {
            for (oa, ia) in [(1, 1), (0, 2)] {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue;
                    }
                    let p = Params {
                        swap,
                        outer_a: oa,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia} {swap}: {e}"));
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                }
            }
        }
    }
}

/// Whether every `(bundle, fault)` is rejected by the harness on at least one of the specs.
fn excl_mutants_rejected(cases: &[(&str, &[u8])]) {
    for &(tw, faults) in cases {
        for &fault in faults {
            let caught = [
                (spec6(), WIDE6),
                (spec4(), WIDE),
                (spec3(), P),
                (spec5(), P),
            ]
            .into_iter()
            .any(|(s, base)| {
                [(1usize, 1usize), (0, 2), (2, 1), (3, 1)]
                    .iter()
                    .any(|&(oa, ia)| {
                        let p = Params {
                            outer_a: oa,
                            inner_a: ia,
                            tw: Tweaks::parse(tw),
                            ..base
                        };
                        onehot::FAULT.with(|c| c.set(fault));
                        let (lm, ops, _) = build(&s, p);
                        onehot::FAULT.with(|c| c.set(0));
                        eval(&s, p, &lm, &ops, 1 << 12).is_err()
                    })
            });
            assert!(caught, "{tw}: fault {fault} must be rejected");
        }
    }
}

/// Lever `V` (aligned item one-hot): end to end, alone and with `K`, the split one-hots, `z`, `f`.
#[test]
fn aligned_item_hot_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxHV",
        "imchxHKV",
        "imchxgrHV",
        "imchxgrdkHKV",
        "imchxgrdky3zfabHKV",
        "imchxgrdky5zfabHV",
    ]);
}

/// Lever `V`'s mutants: 35 (a boundary slot written by the plain leaf flag) and the item one-hot's
/// read / erasure faults 31-33 on the aligned layout, and 34 (`K`'s clear) with it.
#[test]
fn aligned_item_hot_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrHV", &[35, 31, 32, 33]),
        ("imchxgrdky3zfabHKV", &[35, 31, 32, 34]),
    ]);
}

/// Lever `I` (in-place expansion and measured collapse of the aligned item one-hot): end to end.
#[test]
fn inplace_item_hot_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxHVI",
        "imchxHKVI",
        "imchxgrdkHKVI",
        "imchxgrdky3zfabHKVI",
        "imchxgrdky5zfabHVI",
    ]);
}

/// Lever `I`'s mutants (36: a collapse `CZ` skipped; 37: the wrong index bit in it) and `K`'s
/// clear (34) with it, each rejected by the harness.
#[test]
fn inplace_item_hot_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrHVI", &[36, 37, 32, 33]),
        ("imchxgrdky3zfabHKVI", &[36, 37, 34]),
    ]);
}

/// Lever `Z` (shared triple corrections for an odd group-bit count): end to end with `G = 4`
/// and `G = 6`, with the fold, the measured unload and the checkpoint (whose fan-out goes
/// through the same corrections).
#[test]
fn shared_triple_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky4zZ",
        "imchxgrdky4zfaZ",
        "imchxgrdky4zfabZ",
        "imchxgrdky6zfabZ",
        "imchxgrdky4zfabHVIZ",
    ]);
}

/// Lever `Z`'s mutants (38: a product's `f f'` fan-out skipped; 39: the shared product left out
/// of the second bit) are rejected by the harness.
#[test]
fn shared_triple_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdky4zZ", &[38, 39]),
        ("imchxgrdky4zfabZ", &[38, 39]),
    ]);
}

/// Lever `X` (the outer read's exclusive multiplexer): end to end, alone and on the elites.
#[test]
fn excl_select_passes_on_exact_specs() {
    // Two and three outer index bits in blocks (a pair plus the lone block; a shared triple and
    // two pairs), besides `excl_bundles_pass`'s 0 and 1.
    for oa in [2usize, 3] {
        for (s, base) in [(spec6(), WIDE6), (spec4(), WIDE), (spec5(), P)] {
            for tw in ["imchxX", "imchxgrdky3zfaHKVIX"] {
                let p = Params {
                    outer_a: oa,
                    inner_a: 1,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev =
                    eval(&s, p, &lm, &ops, 1 << 13).unwrap_or_else(|e| panic!("{tw} oa {oa}: {e}"));
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / f64::from(1u32 << 13)).sqrt() + 1e-9;
                assert!((ev.toffoli - want).abs() <= tol, "{tw} oa {oa}");
            }
        }
    }
    excl_bundles_pass(&[
        "imchxX",
        "imchxgrdkX",
        "imchxgrdky3zfaHKVIX",
        "imchxgrdky5zfabHVIX",
    ]);
}

/// Lever `X`'s mutants (40: a pair product's register CNOT skipped; 41: the low one-hot's
/// measured erasure without its `CZ`) are rejected by the harness.
#[test]
fn excl_select_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdkX", &[40, 41]),
        ("imchxgrdky3zfaHKVIX", &[40, 41]),
    ]);
}

/// Lever `C` (the RPREP one-hot in aligned classes, written in place and erased backwards by
/// measurement): end to end with G = 2, 3, 4, with and without the checkpoint, on the elite.
#[test]
fn class_inplace_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgr2C",
        "imchxgrdky3zaC",
        "imchxgrdky3zabC",
        "imchxgrdky4zabCZ",
        "imchxgrdky3zabCHKVIX",
    ]);
}

/// Lever `C` with the one-body rows split into virtual rows by their top 1 or 2 lo bits (the
/// depth the layout picks on Li), forced on the exact specs: end to end, and the sub-row
/// collapse's mutant (44) rejected.
#[test]
fn class_inplace_split_rows_pass_on_exact_specs() {
    for s_ob in [1usize, 2] {
        onehot::FORCE_S_OB.with(|c| c.set(Some(s_ob)));
        let r = std::panic::catch_unwind(|| {
            excl_bundles_pass(&["imchxgrdky3zaC", "imchxgrdky3zabC", "imchxgrdky5zabCHVIX"]);
            excl_mutants_rejected(&[("imchxgrdky3zabC", &[44])]);
        });
        onehot::FORCE_S_OB.with(|c| c.set(None));
        assert!(r.is_ok(), "split depth {s_ob}");
    }
}

/// Lever `C`'s mutants (42: a row fold's `CZ` skipped; 43: a `lo` collapse `CZ` skipped) are
/// rejected by the harness.
#[test]
fn class_inplace_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdky3zaC", &[42, 43]),
        ("imchxgrdky3zabCHKVIX", &[42, 43]),
    ]);
}

/// Lever `J` (the checkpoint's `id'` made before the RPREP write, erased after RPREP^dagger):
/// end to end with the fold and with lever `C`, and its mutant 45 (the gated `id'` phase dropped).
#[test]
fn early_id_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky3zfabJ",
        "imchxgrdky3zabCJ",
        "imchxgrdky5zabCHVIXJ",
    ]);
    excl_mutants_rejected(&[
        ("imchxgrdky3zfabJ", &[45]),
        ("imchxgrdky3zabCJ", &[45, 25, 26]),
    ]);
}

/// Lever `D` (the one-body lanes' low item bits and `pos_e` stashed in the read's output during
/// the item one-hot's life): end to end with and without `V` and `I`, and mutant 46 (the unstash
/// Toffoli dropped) rejected.
#[test]
fn stash_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdkHKD",
        "imchxgrdkHKVD",
        "imchxgrdkHKVID",
        "imchxgrdky3zabCHKVIXD",
        "imchxgrdky3zabCHKVIXDJ",
    ]);
    excl_mutants_rejected(&[("imchxgrdkHKVID", &[46]), ("imchxgrdkHKD", &[46])]);
}
/// Lever `W`'s self-bucket term: dropping the two outcomes' `(-1)^(m self(x_o))` (fault 46) is
/// rejected on every test spec whose outer table has a self-aliased bucket (`alt[i] = i`), and
/// the test reports which specs have one (on the others the fault is vacuous).
#[test]
fn outer_witness_self_buckets() {
    let mut caught = 0;
    for (s, base, name) in [
        (spec3(), P, "N=3"),
        (spec5(), P, "N=5"),
        (spec4(), WIDE, "N=4 wide"),
        (spec6(), WIDE6, "R=6"),
    ] {
        let p = Params {
            tw: Tweaks::parse("imchxgrdky3zfabSRNOBW"),
            ..base
        };
        let map = lane_map(&s, p).unwrap();
        let items = s.outer_items();
        let selfs = (0..items)
            .filter(|&i| map.outer.alt[i] as usize == i)
            .count();
        onehot::FAULT.with(|c| c.set(46));
        let (lm, ops, _) = build(&s, p);
        onehot::FAULT.with(|c| c.set(0));
        let rejected = eval(&s, p, &lm, &ops, 1 << 12).is_err();
        println!("{name}: {selfs} self-aliased outer buckets; fault 46 rejected: {rejected}");
        if selfs > 0 {
            assert!(rejected, "{name}: fault 46 must be rejected");
            caught += 1;
        }
    }
    assert!(caught > 0, "no test spec has a self-aliased outer bucket");
}

/// Lever `Q` under the measured-unload bundle: the folded
/// write's mutants (22: class flags left out of the fixup, 23: class 0's slots shifted) stay
/// rejected when the flags are measured early. On the six-row spec only: the other test specs
/// have no folded class at G = 3, so the faults are vacuous there.
#[test]
fn early_flags_mutants_are_rejected() {
    let s = spec6();
    for tw in ["imchxgrdky3zfabSRNOBWQ", "imchxgrd3zfQ"] {
        for fault in [22u8, 23] {
            let p = Params {
                tw: Tweaks::parse(tw),
                ..WIDE6
            };
            onehot::FAULT.with(|c| c.set(fault));
            let (lm, ops, _) = build(&s, p);
            onehot::FAULT.with(|c| c.set(0));
            assert!(
                eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                "{tw}: fault {fault} must be rejected"
            );
        }
    }
}

// ---- non-rotation levers ---------------------------------------------------------------------

/// Lever `t` (the inner signs applied in the control-rooted item-one-hot read, one
/// word bit `delta = s_own ^ s_alt` in place of the two sign flags): end to end on the six-row,
/// wide N = 4 and exact N = 3 / N = 5 specs, with and without `V`, `I`, `K`, `D`, the split
/// one-hots, `z`, `Z`, `C`, `R` and the rest of the measured-unload bundle.
const SIGN_PASS: [&str; 13] = [
    "imchxgrdky5zabCHVIXJtRNOBWPs",
    "imchxgrdky4zabCAHVIXZJtRNOBWPYs",
    "imchxgrdy4zabCAHVIXZJtBPYsU",
    "imchxgrdky3zabCHKVIXDtRNBWUs",
    "imchxgrdkyHt",
    "imchxgrdkyHKt",
    "imchxgrdky3zfabHt",
    "imchxgrdky3zfabHKtRNOB",
    "imchxgrdky5zfabHtRNOBW",
    "imchxgrdky3zabCHKVIXDt",
    "imchxgrdky4zabCAHVIXZJtRNOBW",
    "imchxgrdky5zabCHVIXJtRNOBW",
    "imchxgrdky3zabCHKVIXDtRNOBW",
];

#[test]
fn sign_pass_passes_on_exact_specs() {
    excl_bundles_pass(&SIGN_PASS);
}

/// What `t` saves per copy against `S` on the same bundle (static ledger, exact): the alt sign
/// flag's read (one Toffoli per inner index whose word has it), and `S`'s two ANDs; the root
/// costs at most one Toffoli per pass.
#[test]
fn sign_pass_costs_what_it_claims() {
    let s = spec6();
    let copy = |tw: &str| {
        let p = Params {
            tw: Tweaks::parse(tw),
            ..WIDE6
        };
        build(&s, p).2.sum("copy").0
    };
    for base in ["imchxgrdky3zfabH", "imchxgrdky3zabCHKVIXD"] {
        let with_s = copy(&format!("{base}S"));
        let with_t = copy(&format!("{base}t"));
        let n_i = 1u64 << WIDE6.inner.0;
        assert!(with_t < with_s, "{base}: t {with_t} vs S {with_s}");
        assert!(
            with_s - with_t <= n_i + 2,
            "{base}: t {with_t} vs S {with_s}"
        );
    }
}

/// Lever `t`'s mutants, each rejected by the harness on at least one spec: the in-pass sign `CZ`
/// skipped (90), `CZ(lt, delta)` skipped (92), `delta` stored as 0 (93), the read and its erasure
/// not rooted at the control (94: control-0 lanes then pick up the sign and their own reads), the
/// alt slot's `i` part not cancelled (95). Fault 91 (the in-pass sign's left-over slot phases
/// skipped) is vacuous on these specs (no slot holds two items with the sign set at one inner
/// index); `itemhot::rooted_tests` catches it exhaustively on random tables.
#[test]
fn sign_pass_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdkyHt", &[90, 92, 93, 94, 95]),
        ("imchxgrdky3zabCHKVIXDt", &[90, 92, 93, 94, 95]),
        ("imchxgrdky4zabCAHVIXZJtRNOBW", &[90, 92, 94, 95]),
    ]);
}

#[test]
fn sign_pass_letter_parses() {
    let t = Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBW");
    assert!(t.sign_pass && !t.sign_phase && t.item_hot && t.lt_release);
    assert!(!Tweaks::parse("all").sign_pass);
    assert!(!Tweaks::parse("imchxgrdky4zabCAHVIXZJSRNOBW").sign_pass);
}

// ---- lever P ---------------------------------------------------------------------------------

/// Lever `P` (the last rotation's `pi` bit applied at the chain's edge as `Z_p Z_q` from the
/// one-hot): end to end on the exact specs, with the unsplit, paired, shared-triple and class
/// one-hots, the item one-hot and the measured-unload levers.
#[test]
fn pi_edge_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrP",
        "imchxgrdky3zabCP",
        "imchxgrdky4zabCZP",
        "imchxgrdky3zabCHKVIXDP",
        "imchxgrdky5zabCAHVIXJP",
        "imchxgrdky4zabCAHVIXZJSRNOBWP",
    ]);
}

/// Lever `P`'s mutants (47: the edge phase left out; 48: the phase on the pair's second mode left
/// out) are rejected by the harness.
#[test]
fn pi_edge_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrP", &[47, 48]),
        ("imchxgrdky3zabCP", &[47, 48]),
        ("imchxgrdky4zabCZP", &[47, 48]),
    ]);
}

/// Lever `P` acts on the test specs (some spec's last rotation uses the `pi` bit, so the ops
/// change) and leaves every bundle without the letter alone.
#[test]
fn pi_edge_is_not_vacuous() {
    let mut changed = 0;
    for (s, base) in [
        (spec6(), WIDE6),
        (spec4(), WIDE),
        (spec5(), P),
        (spec3(), P),
    ] {
        for tw in ["imchxgr", "imchxgrdky3zabC"] {
            let p0 = Params {
                tw: Tweaks::parse(tw),
                ..base
            };
            let p1 = Params {
                tw: Tweaks::parse(&format!("{tw}P")),
                ..base
            };
            let (_, a, _) = build(&s, p0);
            let (_, b, _) = build(&s, p1);
            if a.len() != b.len()
                || a.iter()
                    .zip(&b)
                    .any(|(x, y)| format!("{x:?}") != format!("{y:?}"))
            {
                changed += 1;
            }
        }
    }
    assert!(
        changed > 0,
        "lever P never changed the ops on the test specs"
    );
    assert!(!Tweaks::parse("imchxgrdky4zabCAHVIXZJSRNOBW").pi_edge);
    assert!(Tweaks::parse("imchxgrdky4zabCAHVIXZJSRNOBWP").pi_edge);
    assert!(!Tweaks::parse("all").pi_edge);
}

/// Lever `Y` (the shared triple's common product by a CNOT sandwich, no scratch): end to end with
/// `G = 4` and `G = 6`, alone and on the Li composition; its mutant 49 (the sandwich's first CNOT
/// dropped) rejected by the harness.
#[test]
fn triple_sandwich_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky4zZY",
        "imchxgrdky4zabCZY",
        "imchxgrdky6zfabZY",
        "imchxgrdky4zabCAHVIXZJSRNOBWPY",
    ]);
    excl_mutants_rejected(&[("imchxgrdky4zZY", &[49]), ("imchxgrdky4zabCZY", &[49])]);
}

// ---- rotation levers -------------------------------------------------------------------------

/// How many networks (squares and one-body) of `s` lever `s` flips: the end-to-end checks below
/// are only meaningful if some leaf is delivered as `-u`.
fn flipped_leaves(s: &SaSpec) -> usize {
    s.nets
        .iter()
        .chain(&s.e_nets)
        .filter(|n| !std::sync::Arc::ptr_eq(n, &super::signnorm::normalized(n)))
        .count()
}

/// Lever `s` (global sign normalisation of the Householder vectors): end to end on the exact and
/// wide specs, alone and on the Li and Reiher elites (`G = 3, 4, 5`, with `C`, `X`, `Z`, `J`, `D`).
#[test]
fn sign_norm_passes_on_exact_specs() {
    let flips: Vec<usize> = [spec6(), spec4(), spec5(), spec3()]
        .iter()
        .map(flipped_leaves)
        .collect();
    println!("leaves flipped per spec (R=6, N=4 wide, N=5, N=3): {flips:?}");
    assert!(flips.iter().filter(|&&f| f > 0).count() >= 2, "{flips:?}");
    excl_bundles_pass(&[
        "imchxgrs",
        "imchxgrdky3zfabs",
        "imchxgrdky3zabCHKVIXDs",
        "imchxgrdky4zabCHVIXZJs",
        "imchxgrdky5zabCHVIXJs",
    ]);
}

/// Lever `s`'s mutant (80: the last angle's half turn left out, so a flipped leaf's network maps
/// orbital 0 to neither `u` nor `-u`) is rejected by the harness.
#[test]
fn sign_norm_mutants_are_rejected() {
    excl_mutants_rejected(&[("imchxgrs", &[80]), ("imchxgrdky5zabCHVIXJs", &[80])]);
}

/// Lever `s` changes no circuit without the letter: every elite without `s` emits the same ops
/// (`tests/sa_digests.rs` pins the measured ones); with it, only the angle
/// tables differ, so the ledger moves by the top-bit corrections alone.
#[test]
fn sign_norm_letter_parses() {
    assert!(Tweaks::parse("imchxgrs").sign_norm);
    assert!(!Tweaks::parse("all").sign_norm);
    assert!(!Tweaks::parse("imchxgrdky5zabCHVIXJ").sign_norm);
}
/// The six-row spec with every rotation but the last below `pi` (top bit clear), so
/// the register widths differ (`BETA - 1` against `BETA` for the last), as on the pinned specs.
fn spec6_narrow() -> SaSpec {
    let (n, r, bb, c) = (4usize, 6usize, 3usize, 1usize);
    let wide = payload(
        n,
        (r, bb, c),
        &[1.0, -2.0, 1.0, 4.0],
        &[1.0, -4.0, 2.0, 1.0, -1.0, 3.0],
        &[
            1.0, -1.0, 2.0, 1.0, 2.0, -1.0, -1.0, 1.0, 1.0, 2.0, 1.0, -2.0, 1.0, 1.0, -1.0, 4.0,
            -1.0, 2.0,
        ],
    );
    // Header 8 + 7 u32 + f64 sos; e[N]; e_angles[N][N-1]; wB[R][C]; w[R][C][B]; angles[R][B][N-1].
    let mut bytes = wide;
    let e_ang = 8 + 28 + 8 + 8 * n;
    let ang = e_ang + 4 * n * (n - 1) + 8 * r * c + 8 * r * c * bb;
    let mask = |bytes: &mut Vec<u8>, at: usize, count: usize| {
        for i in 0..count {
            if i % (n - 1) != n - 2 {
                bytes[at + 4 * i] &= 0x7f;
            } else {
                bytes[at + 4 * i] |= 0x80;
            }
        }
    };
    mask(&mut bytes, e_ang, n * (n - 1));
    mask(&mut bytes, ang, r * bb * (n - 1));
    assert_eq!(bytes.len(), ang + 4 * r * bb * (n - 1));
    parse_payload("test-sa6n-v1", &bytes).unwrap()
}

/// Lever `U`: the split one-hot's angle register stays loaded across the Majorana
/// with the measured unload, and bits above a rotation's width are reset once the pass leaves a
/// wider rotation. End to end on the exact and wide toy specs and on a spec whose widths differ.
#[test]
fn hold_majorana_passes_on_exact_specs() {
    let s = spec6_narrow();
    let map = super::lane_map(&s, WIDE6).unwrap();
    let t = super::tables::SaTables::with_items(&s, &map, true);
    assert!(
        t.widths[..t.widths.len() - 1]
            .iter()
            .all(|&w| w < t.widths[t.widths.len() - 1]),
        "the narrow spec must have a wider last rotation: {:?}",
        t.widths
    );
    excl_bundles_pass(&[
        "imchxgrdky3zabU",
        "imchxgrdky3zabCHKVIXDU",
        "imchxgrdky3zabCHKVIXDSRNBWU",
        "imchxgrdky5zabCHVIXJSRNBWU",
    ]);
    let samples = 1 << 13;
    for tw in [
        "imchxgrdky3zabU",
        "imchxgrdky3zabCHKVIXDSRNBWU",
        "imchxgrdky5zabCHVIXJSRNBWU",
    ] {
        for (oa, ia) in [(1, 1), (0, 2)] {
            let p = Params {
                outer_a: oa,
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..WIDE6
            };
            let (lm, ops, led) = build(&s, p);
            let var = gated_var_bound(&ops);
            let ev = eval(&s, p, &lm, &ops, samples)
                .unwrap_or_else(|e| panic!("narrow {tw} oa {oa} ia {ia}: {e}"));
            let (want, _) = ledger_c_step(&led, &s, p);
            let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
            assert!(
                (ev.toffoli - want).abs() <= tol,
                "narrow {tw}: {} vs {want}",
                ev.toffoli
            );
            // `P` never costs Toffolis against the same bundle without it.
            let q = Params {
                tw: Tweaks::parse(&tw.replace('U', "")),
                ..p
            };
            let (_, _, led0) = build(&s, q);
            assert!(
                led.sum("copy").0 <= led0.sum("copy").0,
                "{tw}: U costs Toffolis"
            );
            // Mutant 60 (one register bit reset while live) is rejected.
            onehot::FAULT.with(|c| c.set(60));
            let (lm, ops, _) = build(&s, p);
            onehot::FAULT.with(|c| c.set(0));
            assert!(
                eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                "{tw} oa {oa} ia {ia}: fault 60 must be rejected"
            );
        }
    }
}

/// Lever `T` (four-group aligned item one-hot): end to end through the harness on
/// the toy specs, with and without `K`, `D`, the measured-unload letters and `P`, at G = 3 / 4 /
/// 5; and its read / phase / collapse mutants (61, 62, 36, 37) are rejected.
#[test]
fn item_groups4_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxHVIT",
        "imchxgrdkHKVIT",
        "imchxgrdky3zabCHKVIXT",
        "imchxgrdky3zabCHKVIXDSRNBWUT",
        "imchxgrdky4zZabCHKVIXSRNOBWT",
        "imchxgrdky5zabCHKVIXSRNOBWUT",
    ]);
    excl_mutants_rejected(&[
        ("imchxgrdky3zabCHKVIXDSRNBWUT", &[61, 62, 36, 37]),
        ("imchxgrdkHKVIT", &[61, 62, 36]),
    ]);
}

/// Lever `G` (the paired unary lookup), `M` (its low index qubits host
/// slots) and `e` (`hi`, `is_ob` released across the read and reloaded from the item), end to end
/// on the four exact specs, every slot width the index allows (`inner_a` is `s` with `G`), both
/// SELECTs, with every axis verified and the harness's Toffolis equal to the ledger's.
const LI_WIDTH: [&str; 13] = [
    "imchxGF",
    "imchxMenF",
    "imchxgrdky4zabCAXZJSRNOBWMenF",
    "imchxG",
    "imchxM",
    "imchxMe",
    "imchxGn",
    "imchxMn",
    "imchxgrdky4zabCAXZJSRNOBWMen",
    "imchxgrdkMe",
    "imchxgrdky3zfabMe",
    "imchxgrdky4zabCAXZJSRNOBWMe",
    "imchxgrdky5zabCAXJSRNOBWGe",
];

#[test]
fn li_width_levers_pass_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base) in [
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
        (spec3(), "N=3", P),
    ] {
        let map = lane_map(&s, base).expect("lane map");
        let bits = map.inner[0].k as usize + super::tables::SaTables::new(&s, &map).kx;
        for tw in LI_WIDTH {
            for ia in 1..bits {
                for swap in [true, false] {
                    if !swap && tw.contains('y') {
                        continue;
                    }
                    let p = Params {
                        swap,
                        outer_a: 1,
                        inner_a: ia,
                        tw: Tweaks::parse(tw),
                        ..base
                    };
                    let (lm, ops, led) = build(&s, p);
                    let var = gated_var_bound(&ops);
                    let ev = eval(&s, p, &lm, &ops, samples)
                        .unwrap_or_else(|e| panic!("{name} {tw} s {ia} {swap}: {e}"));
                    assert!(ev.facts.nested_validated);
                    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                        assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                    }
                    let (want, _) = ledger_c_step(&led, &s, p);
                    let tol = 6.0 * (var / samples as f64).sqrt() + 1e-9;
                    assert!(
                        (ev.toffoli - want).abs() <= tol,
                        "{name} {tw} s {ia}: harness {} vs ledger {want} (tol {tol})",
                        ev.toffoli
                    );
                }
            }
        }
    }
}

/// Paired-lookup mutants, each rejected by the harness on at least one exact spec: the paired lookup's
/// own faults (paired.rs: 40 `X Y` fan-out, 41 parities swapped, 42 slot fixup; 43 an index bit
/// not rebuilt, 44 a hosted slot not moved back, both with `U`) and lever `e`'s (onehot.rs: 47
/// the reload's `Z` fixup skipped, 48 the reloaded value not swapped in).
#[test]
fn li_width_mutants_are_rejected() {
    let cases: [(&str, u8, bool); 10] = [
        ("imchxgrdkMnF", 46, true),
        ("imchxgrdkMn", 45, true),
        ("imchxgrdkMen", 47, false),
        ("imchxgrdkG", 40, true),
        ("imchxgrdkG", 41, true),
        ("imchxgrdkG", 42, true),
        ("imchxgrdkM", 43, true),
        ("imchxgrdkM", 44, true),
        ("imchxgrdkMe", 47, false),
        ("imchxgrdkMe", 48, false),
    ];
    for (tw, fault, in_paired) in cases {
        let caught = [
            (spec6(), WIDE6),
            (spec4(), WIDE),
            (spec3(), P),
            (spec5(), P),
        ]
        .into_iter()
        .any(|(s, base)| {
            (1..4usize).any(|ia| {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                if in_paired {
                    super::paired::FAULT.with(|c| c.set(fault));
                } else {
                    onehot::FAULT.with(|c| c.set(fault));
                }
                let r = std::panic::catch_unwind(|| build(&s, p));
                super::paired::FAULT.with(|c| c.set(0));
                onehot::FAULT.with(|c| c.set(0));
                match r {
                    Err(_) => true,
                    Ok((lm, ops, _)) => eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                }
            })
        });
        assert!(caught, "{tw}: fault {fault} must be rejected");
    }
}

#[test]
fn li_width_letters_parse() {
    let t = Tweaks::parse("imchxgrdky4zabCAXZJSRNOBWMe");
    assert!(t.paired_lookup && t.slot_host && t.field_release);
    let t = Tweaks::parse("imchxG");
    assert!(t.paired_lookup && !t.slot_host && !t.field_release && !t.slot_collapse);
    assert!(Tweaks::parse("imchxGn").slot_collapse);
    assert_eq!(Tweaks::parse("none"), Tweaks::OFF);
    for old in [
        "imchxgrdky5zabCHVIXJ",
        "imchxgrdky4zfabHVIZ",
        "all",
        "imchxL",
        "none",
    ] {
        let t = Tweaks::parse(old);
        assert!(
            !t.paired_lookup && !t.slot_host && !t.field_release && !t.slot_collapse,
            "{old}"
        );
    }
}
/// Lever `o` (the class one-hot packed by length, square rows split too): end to end on the
/// exact and wide specs, at every forced pair of split depths `(square, one-body)` (so the
/// identity lanes of split square rows land in fresh sub-rows, in wider classes and on aliased
/// leaves), and at the depths `best_pack` picks.
#[test]
fn class_pack_passes_on_exact_specs() {
    let bundles = [
        "imchxgrdky3zabCo",
        "imchxgrdky4zabCZo",
        "imchxgrdky3zabCHKVIXDso",
        "imchxgrdky5zabCHVIXJso",
    ];
    excl_bundles_pass(&bundles);
    for sq in 0..=2usize {
        for ob in 0..=2usize {
            onehot::FORCE_PACK.with(|c| c.set(Some((sq, ob))));
            let r = std::panic::catch_unwind(|| excl_bundles_pass(&bundles[..2]));
            onehot::FORCE_PACK.with(|c| c.set(None));
            assert!(r.is_ok(), "forced depths ({sq}, {ob})");
        }
    }
}

/// Lever `o`'s mutants: 81 (an identity lane's stand-in leaf is the row's leaf 0, not the
/// checkpoint's `L`), and lever `C`'s fold, collapse and sub-row faults 42-44 on the packed layout.
#[test]
fn class_pack_mutants_are_rejected() {
    // At depth (2, 2) = k_i every class is one slot wide: no expansion for fault 43 to break.
    for (sq, ob, faults) in [
        (1usize, 1usize, &[42u8, 43, 44][..]),
        (2, 2, &[42, 44][..]),
        (1, 0, &[42, 43, 44][..]),
    ] {
        onehot::FORCE_PACK.with(|c| c.set(Some((sq, ob))));
        let r = std::panic::catch_unwind(|| {
            excl_mutants_rejected(&[
                ("imchxgrdky3zabCo", faults),
                ("imchxgrdky3zabCHKVIXDso", &[42]),
            ]);
        });
        onehot::FORCE_PACK.with(|c| c.set(None));
        assert!(r.is_ok(), "forced depths ({sq}, {ob})");
    }
    onehot::FORCE_PACK.with(|c| c.set(Some((1, 1))));
    let r = std::panic::catch_unwind(|| excl_mutants_rejected(&[("imchxgrdky3zabCo", &[81])]));
    onehot::FORCE_PACK.with(|c| c.set(None));
    assert!(r.is_ok(), "fault 81");
}

// ---- lever `q<b>` (rank-scheduled angle delivery) --------------------------------------------

/// Lever `q<b>`: end to end on the six-row, wide N = 4 and exact N = 3 / N = 5 specs, with G = 2
/// and 3, budgets from one rotation's word rank to the whole span, with and without the
/// exclusivity / measured-unload / sign letters: verified axes and the ledger equal to the
/// harness (the gadget is not outcome-gated, so exactly).
#[test]
fn rank_delivery_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky3zabq8",
        "imchxgrdky3zabq11",
        "imchxgrdky3zabq64",
        "imchxgrdky2zabq9",
        "imchxgrdky2abq12",
        "imchxgrdky3zabCHKVIXDq10",
        "imchxgrdky3zabCHKVIXDSRNBWq14",
        "imchxgrdky3zabCHKVIDXtNBWsq9",
        "imchxgrdky3zabCHKVIDXtNBWsq13",
        "imchxgrdky3zabq8_1",
        "imchxgrdky3zabq11_1",
        "imchxgrdky2zabq9_1",
        "imchxgrdky3zabCHKVIXDq10_1",
        "imchxgrdky3zabCHKVIDXtNBWsq9_1",
        "imchxgrdky3zabCHKVIDXtNBWsq13_1",
    ]);
}

/// Lever `q<b>`: its mutants (90 a buy's group fan skipped, 91 a measured unload's fixup term
/// skipped, 92 a retarget's base fix skipped on bit 0, 93 a CNOT gathering the kept
/// intersection skipped, 94 the identity lanes' non-leaf one-hot states left out of the exact
/// set) are rejected by the harness.
#[test]
fn rank_delivery_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdky3zabCHKVIXDq10", &[90, 91, 92, 93, 94]),
        ("imchxgrdky2zabq9", &[90, 91, 92]),
        ("imchxgrdky3zabCHKVIXDq10_1", &[90, 91, 92, 93, 94]),
    ]);
}

/// Lever `q<b>_1.<c>` (the held span cut to `c` dimensions across the Majorana),
/// end to end on the exact specs, budgets 1 .. above the held span:
/// verified axes and the ledger equal to the harness. The Majorana-gap mutants 91 / 93 (inside the
/// cut's measured unload and gathering) are rejected.
#[test]
fn rank_park_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky3zabq11_1.1",
        "imchxgrdky3zabq11_1.4",
        "imchxgrdky2zabq9_1.2",
        "imchxgrdky3zabCHKVIXDq10_1.3",
        "imchxgrdky3zabCHKVIDXtNBWsq13_1.2",
        "imchxgrdky3zabCHKVIDXtNBWs+q13_1.5",
        "imchxgrdky3zabCHKVIDXtNBWsq13_1.64",
        "imchxgrdky4zabq8_1.3",
    ]);
    excl_mutants_rejected(&[("imchxgrdky3zabCHKVIXDq10_1.2", &[91, 93])]);
}

/// `.<c>` parses only after `q<b>_<mode>`; a budget at or above the held span costs
/// no Toffoli.
#[test]
fn rank_park_letter_parses() {
    let t = Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs+q16_1.13");
    assert_eq!((t.rank_hold, t.rank_mode, t.rank_park), (16, 1, 13));
    assert_eq!(
        Tweaks { rank_park: 0, ..t },
        Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs+q16_1")
    );
    assert_eq!(Tweaks::parse("all").rank_park, 0);
    let s = spec5();
    let p = |tw: &'static str| Params {
        outer_a: 1,
        inner_a: 1,
        tw: Tweaks::parse(tw),
        ..P
    };
    // A budget above the span keeps every direction (the cut only measures out redundant or
    // pure one-hot rows), so the Toffolis are unchanged.
    let (_, _, la) = build(&s, p("imchxgrdky3zabq11_1"));
    let (_, _, lb) = build(&s, p("imchxgrdky3zabq11_1.64"));
    let (ca, _) = ledger_c_step(&la, &s, p("imchxgrdky3zabq11_1"));
    let (cb, _) = ledger_c_step(&lb, &s, p("imchxgrdky3zabq11_1.64"));
    assert!((ca - cb).abs() < 1e-9, "{ca} vs {cb}");
}

/// Lever `q<b>_1` on a split one-hot of `G = 4 .. 6` groups (pair products,
/// lever `Z`'s triple shared by two bought directions when `G - 1` is odd), end to end.
#[test]
fn rank_delivery_wide_groups_pass_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky4zabq8_1",
        "imchxgrdky4zabZq12_1",
        "imchxgrdky5zabq9_1",
        "imchxgrdky6zabZq10_1",
        "imchxgrdky4zabCHVIXZJq10_1",
        "imchxgrdky4zabCHVIXZJsYq64_1",
        "imchxgrdky4zabq9",
    ]);
}

/// Mutants 120 (the triple's shared CNOT skipped) and 121 (the triple's `f2`
/// written into the first register), and 90-93 at `G = 4`, are rejected by the harness.
#[test]
fn rank_delivery_wide_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdky4zabq8_1", &[90, 91, 92, 93, 120, 121]),
        ("imchxgrdky6zabZq10_1", &[120, 121]),
    ]);
}

#[test]
fn rank_delivery_letter_parses() {
    let t = Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWsq150");
    assert_eq!(t.rank_hold, 150);
    assert_eq!(t.hot_groups, 3);
    assert_eq!(
        Tweaks { rank_hold: 0, ..t },
        Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs")
    );
    assert_eq!(Tweaks::parse("q12imchxgr3za").hot_groups, 3);
    assert_eq!(Tweaks::parse("all").rank_hold, 0);
    let t1 = Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWsq150_1");
    assert_eq!((t1.rank_hold, t1.rank_mode), (150, 1));
    assert_eq!(Tweaks { rank_mode: 0, ..t1 }, t);
    assert_eq!(Tweaks::parse("none").rank_hold, 0);
}

// ---- lever + -----------------------------------------------------------------------------------

/// An exact spec with real inner padding: `(R, B, C) = (3, 5, 2)`, six inner items in eight
/// buckets (two padding buckets), each square's weights summing to 16 so every count is exact:
/// a dominant item that feeds both padding buckets (q0, q4), one whose largest item feeds only
/// one (q1: the second goes elsewhere, so a padding row survives the offset), one fed exactly
/// (q2: its own bucket then aliases out entirely), the identity item dominant (q3, q5), a zero
/// weight. Outer weights `2 |e_r|` and `S^2 / 2 = 128` sum to 1,024.
fn spec7() -> SaSpec {
    let bytes = payload(
        4,
        (3, 5, 2),
        &[32.0, -16.0, 64.0, 16.0],
        &[2.0, -3.0, 2.0, 8.0, 1.0, 8.0],
        &[
            8.0, -1.0, 2.0, -2.0, 1.0, // q0
            3.0, -3.0, 3.0, 2.0, -2.0, // q1
            -4.0, 4.0, 4.0, 1.0, 1.0, // q2
            1.0, 1.0, 1.0, 1.0, 4.0, // q3
            2.0, 2.0, -8.0, 2.0, 1.0, // q4
            0.5, 0.5, 1.0, 0.0, 6.0, // q5
        ],
    );
    parse_payload("test-sa7-v1", &bytes).unwrap()
}

/// `.a` changes only integer alias tables; the existing `+` read and phase-erasure
/// gadget is exercised on a small exact spec. The pinned Li table test checks that `.a`
/// actually aligns the intended bit on its production shape.
#[test]
fn aligned_alias_small_exact_and_offset_mutant() {
    let s = spec7();
    let p = Params {
        tw: Tweaks::parse("imchxgrdkyHt+.a"),
        ..PAD7
    };
    let base = Params {
        tw: Tweaks::parse("imchxgrdkyHt+"),
        ..PAD7
    };
    let m0 = lane_map(&s, base).unwrap();
    let m1 = lane_map(&s, p).unwrap();
    for (a, b) in m0.inner.iter().zip(&m1.inner) {
        assert_eq!(a.counts(6), b.counts(6));
    }
    let (lm, ops, led) = build(&s, p);
    let ev = eval(&s, p, &lm, &ops, 1 << 12).unwrap();
    assert!(ev.facts.nested_validated);
    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
        assert_eq!(
            ev.verdicts.iter().find(|v| v.axis == axis).unwrap().status,
            AxisStatus::Verified,
            "{axis}"
        );
    }
    let (want, _) = ledger_c_step(&led, &s, p);
    let tol = 6.0 * (gated_var_bound(&ops) / 4096.0).sqrt() + 1e-9;
    assert!((ev.toffoli - want).abs() <= tol);
    onehot::FAULT.with(|c| c.set(104));
    let (bad_map, bad_ops, _) = build(&s, p);
    onehot::FAULT.with(|c| c.set(0));
    assert!(eval(&s, p, &bad_map, &bad_ops, 1 << 12).is_err());
}

const PAD7: Params = Params {
    outer: (4, 20),
    inner: (3, 20),
    ..WIDE
};

/// Lever `+` bundles: the H / t compositions of the fronts.
const PAD_OFFSET: [&str; 6] = [
    "imchxgrdkyHt+",
    "imchxgrdky3zabCHKVIXDt+",
    "imchxgrdky3zabCHKVIDXtNBWs+",
    "imchxgrdky5zabCHVIXJtRNOBWPs+",
    "imchxlgrdkyEHKVIXt+",
    "imchxgrdky4zabCAHVIXZJtRNOBWPYs+",
];

/// [`excl_bundles_pass`] on the padded spec (and the four others): every `+` bundle passes
/// `evaluate` end to end with verified axes and the harness's `C_step` equal to the ledger.
#[test]
fn pad_offset_passes_on_exact_specs() {
    excl_bundles_pass(&PAD_OFFSET);
    let s = spec7();
    for &tw in &PAD_OFFSET {
        for (oa, ia) in [(1, 1), (0, 2), (2, 3)] {
            let p = Params {
                outer_a: oa,
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..PAD7
            };
            let (lm, ops, led) = build(&s, p);
            let var = gated_var_bound(&ops);
            let ev = eval(&s, p, &lm, &ops, 1 << 13)
                .unwrap_or_else(|e| panic!("spec7 {tw} oa {oa} ia {ia}: {e}"));
            assert!(ev.facts.nested_validated);
            for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
            }
            let (want, _) = ledger_c_step(&led, &s, p);
            let tol = 6.0 * (var / f64::from(1u32 << 13)).sqrt() + 1e-9;
            assert!(
                (ev.toffoli - want).abs() <= tol,
                "spec7 {tw}: harness {} vs ledger {want} (tol {tol})",
                ev.toffoli
            );
        }
    }
}

/// On the padded spec `+` changes the tables but not the counts, and saves Toffolis in the copy
/// against the same bundle without it (the padding rows the offset zeroes are skipped).
#[test]
fn pad_offset_keeps_counts_and_saves() {
    let s = spec7();
    for base in ["imchxgrdkyHt", "imchxgrdky3zabCHKVIXDt"] {
        let p0 = Params {
            tw: Tweaks::parse(base),
            ..PAD7
        };
        let p1 = Params {
            tw: Tweaks::parse(&format!("{base}+")),
            ..PAD7
        };
        let (m0, m1) = (lane_map(&s, p0).unwrap(), lane_map(&s, p1).unwrap());
        for (t0, t1) in m0.inner.iter().zip(&m1.inner) {
            assert_eq!(t0.counts(8), t1.counts(8));
        }
        assert_eq!(
            m0.rounding_error(&s).unwrap(),
            m1.rounding_error(&s).unwrap()
        );
        let c0 = build(&s, p0).2.sum("copy").0;
        let c1 = build(&s, p1).2.sum("copy").0;
        assert!(c1 < c0, "{base}: + {c1} vs {c0}");
    }
}

/// Lever `+`'s mutants, each rejected by the harness on at least one spec: the offset's paired
/// Toffoli skipped (100), the erasure offset's phases skipped (103: inside `fan_rooted`; 105: the
/// call), the read data not offset (104). (The in-pass sign's item offset `P(item)` is not
/// corrected at all: both copies apply it, so it cancels over the step; 102 is the gadget's.)
#[test]
fn pad_offset_mutants_are_rejected() {
    let s = spec7();
    for (tw, faults) in [
        ("imchxgrdkyHt+", &[100u8, 103, 104, 105][..]),
        ("imchxgrdky3zabCHKVIXDt+", &[100, 103, 104, 105][..]),
    ] {
        for &fault in faults {
            let caught = [(1usize, 1usize), (0, 2), (2, 3)].iter().any(|&(oa, ia)| {
                let p = Params {
                    outer_a: oa,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..PAD7
                };
                onehot::FAULT.with(|c| c.set(fault));
                let (lm, ops, _) = build(&s, p);
                onehot::FAULT.with(|c| c.set(0));
                eval(&s, p, &lm, &ops, 1 << 12).is_err()
            });
            assert!(caught, "{tw}: fault {fault} must be rejected");
        }
    }
}

#[test]
fn pad_offset_letter_parses() {
    assert!(Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs+").pad_offset);
    assert!(!Tweaks::parse("imchxgrdky3zabCHKVIDXtNBWs").pad_offset);
    assert!(!Tweaks::parse("all").pad_offset);
}

// ---- lever t on the paired lookup (G / M with F) -----------------------------------------------

/// Lever `t` (control-rooted read, alt sign in the pass, `delta` word) on the paired
/// lookup and paired phase erasure: the Li low-C and product front compositions plus small ones.
const PAIRED_SIGN: [&str; 6] = [
    "imchxgrdkyGFt",
    "imchxgrdkyMnFt",
    "imchxgrdky3zabCHKVIXDGFt",
    "imchxgrdky4zabCAXZJBMnFsYUt",
    "imchxgrdky4zabCAXZJtRNOBWMenFsY",
    "imchxgrdy4zabCAXZJBMnFsYUt",
];

/// Every bundle passes `evaluate` end to end on the five exact specs at every slot width the
/// index allows, both SELECTs where `y` permits, verified axes, harness `C_step` = ledger.
#[test]
fn paired_sign_pass_passes_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base) in [
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
        (spec3(), "N=3", P),
        (spec7(), "padded", PAD7),
    ] {
        let map = lane_map(&s, base).expect("lane map");
        let bits = map.inner[0].k as usize + super::tables::SaTables::new(&s, &map).kx;
        for tw in PAIRED_SIGN {
            for ia in 1..bits {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples)
                    .unwrap_or_else(|e| panic!("{name} {tw} s {ia}: {e}"));
                assert!(ev.facts.nested_validated);
                for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                    let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                    assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                }
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / 4096.0).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} s {ia}: harness {} vs ledger {want} (tol {tol})",
                    ev.toffoli
                );
            }
        }
    }
}

/// `t` on the paired path saves Toffolis in the copy against the same bundle without it (the
/// sign flag's pairs; `S` / the plain flags otherwise).
#[test]
fn paired_sign_pass_saves() {
    let s = spec7();
    for (base, with) in [
        ("imchxgrdkyGF", "imchxgrdkyGFt"),
        ("imchxgrdky3zabCHKVIXDGF", "imchxgrdky3zabCHKVIXDGFt"),
    ] {
        let copy = |tw: &str| {
            let p = Params {
                inner_a: 3,
                tw: Tweaks::parse(tw),
                ..PAD7
            };
            build(&s, p).2.sum("copy").0
        };
        let (c0, c1) = (copy(base), copy(with));
        assert!(c1 < c0, "{with}: {c1} vs {base}: {c0}");
    }
}

/// The paired sign pass's mutants, each rejected by the harness on at least one spec: the
/// in-pass sign `CZ`s skipped (paired 110), the read not rooted (paired 111), the erasure not
/// rooted (paired 112), and lever `t`'s own `CZ(lt, delta)` skipped (92), `delta` stored as 0 (93),
/// the alt slot's `i` part not cancelled (95).
#[test]
fn paired_sign_pass_mutants_are_rejected() {
    let cases: [(&str, u8, bool); 6] = [
        ("imchxgrdkyGFt", 110, true),
        ("imchxgrdkyGFt", 111, true),
        ("imchxgrdkyGFt", 112, true),
        ("imchxgrdkyGFt", 92, false),
        ("imchxgrdkyGFt", 93, false),
        ("imchxgrdkyMnFt", 95, false),
    ];
    for (tw, fault, in_paired) in cases {
        let caught = [
            (spec7(), PAD7),
            (spec6(), WIDE6),
            (spec4(), WIDE),
            (spec3(), P),
            (spec5(), P),
        ]
        .into_iter()
        .any(|(s, base)| {
            (1..4usize).any(|ia| {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                if in_paired {
                    super::paired::FAULT.with(|c| c.set(fault));
                } else {
                    onehot::FAULT.with(|c| c.set(fault));
                }
                let r = std::panic::catch_unwind(|| build(&s, p));
                super::paired::FAULT.with(|c| c.set(0));
                onehot::FAULT.with(|c| c.set(0));
                match r {
                    Err(_) => false,
                    Ok((lm, ops, _)) => eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                }
            })
        });
        assert!(caught, "{tw}: fault {fault} must be rejected");
    }
}

// ---- lever C on the unsplit one-hot -------------------------------------------------------------

/// Lever `C` with no group digit: the unsplit RPREP one-hot written in place (row one-hot,
/// sub-rows, class expansions with one virtual row per class) and erased by the measured collapse
/// (0 Toffolis; no unfold). End to end on the five exact specs, verified axes, ledger = harness.
const UNSPLIT_C: [&str; 9] = [
    "imchxgrC",
    "imchxgrdkyC",
    "imchxlgrdkyCHKVIXt",
    "imchxlgrdkyCHKVIXt+",
    "imchxgrdkyCAHKVIXtRNOBW",
    "imchxgrEC",
    "imchxgrdkEyC",
    "imchxlgrdkyECHKVIXt+",
    "imchxgrdkyECHKVIXt+",
];

#[test]
fn unsplit_class_passes_on_exact_specs() {
    excl_bundles_pass(&UNSPLIT_C);
    let s = spec7();
    for &tw in &UNSPLIT_C {
        let p = Params {
            outer_a: 1,
            inner_a: 2,
            tw: Tweaks::parse(tw),
            ..PAD7
        };
        let (lm, ops, led) = build(&s, p);
        let ev = eval(&s, p, &lm, &ops, 1 << 12).unwrap_or_else(|e| panic!("spec7 {tw}: {e}"));
        let (want, _) = ledger_c_step(&led, &s, p);
        let tol = 6.0 * (gated_var_bound(&ops) / 4096.0).sqrt() + 1e-9;
        assert!((ev.toffoli - want).abs() <= tol, "spec7 {tw}");
    }
}

/// Its erasure is Clifford: the RPREP^dagger stage costs 0 Toffolis, against the iteration's
/// fixup without `C`.
#[test]
fn unsplit_class_erasure_is_free() {
    let s = spec6();
    for base in ["imchxgr", "imchxgrdky"] {
        let stage = |tw: &str| {
            let p = Params {
                tw: Tweaks::parse(tw),
                ..WIDE6
            };
            let led = build(&s, p).2;
            led.rows
                .iter()
                .zip(&led.expected)
                .find(|(r, _)| r.0.contains("RPREP^dagger"))
                .map(|(_, e)| *e)
                .unwrap()
        };
        assert!(stage(base) > 0.0);
        assert!(stage(&format!("{base}C")).abs() < 1e-12);
    }
}

/// The collapse's mutant (onehot.rs 43: a class split's `CZ` skipped) is rejected on the unsplit
/// layout, and with lever `E` on it so are `E`'s own (14: non-pivot leaves left out of a load; 19:
/// one Gauss-Jordan CNOT dropped).
#[test]
fn unsplit_class_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrC", &[43]),
        ("imchxgrdkyC", &[43]),
        ("imchxgrEC", &[43, 14, 19]),
    ]);
}

// ---- lever `.p` (padding runs: the paired read's pruned slot one-hot) -------------------------

/// The padded spec with two heavier inner weights (rows `q1`, `q2`, first), so that every square table
/// has an item with at least two buckets' worth of lanes: lever `.p` then feeds the padding
/// subcube `[6, 8)` from one donor in every table and the read merges it. (With 3 slot bits the
/// merge saves a slot but loses hosted bit 0: the read stage gains only from 4 slot bits on.)
fn spec8() -> SaSpec {
    let bytes = payload(
        4,
        (3, 5, 2),
        &[32.0, -16.0, 64.0, 16.0],
        &[2.0, -3.0, 2.0, 8.0, 1.0, 8.0],
        &[
            8.0, -1.0, 2.0, -2.0, 1.0, // q0
            6.0, -3.0, 3.0, 2.0, -2.0, // q1
            -8.0, 4.0, 4.0, 1.0, 1.0, // q2
            1.0, 1.0, 1.0, 1.0, 4.0, // q3
            2.0, 2.0, -8.0, 2.0, 1.0, // q4
            0.5, 0.5, 1.0, 0.0, 6.0, // q5
        ],
    );
    parse_payload("test-sa8-v1", &bytes).unwrap()
}

/// [`PAD7`] with more keep bits, for [`spec8`]'s larger 1-norm under the rounding rule.
const PAD8: Params = Params {
    outer: (4, 22),
    inner: (3, 22),
    ..WIDE
};

/// Lever `.p` on the paired lookup (`G` / `M` with the collapse `n`): the Li composition
/// and smaller ones, with and without lever `t`'s rooted read.
const PAD_RUNS: [&str; 5] = [
    "imchxgrdkyGnF.p",
    "imchxgrdkyMnFt.p",
    "imchxgrdky4zabCAXZJBMnFsYUt.p",
    "imchxgrdky4zabCAXZJtRNOBWMenFsY.p",
    "imchxgrdky4zabCAXZJtNOBWMenFsY.p",
];

/// Every `.p` bundle passes `evaluate` end to end on the five exact specs at every slot width
/// the index allows (the padded spec's padding subcube is merged), verified axes, harness
/// `C_step` = ledger.
#[test]
fn pad_runs_passes_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base) in [
        (spec8(), "padded, fed", PAD8),
        (spec7(), "padded", PAD7),
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
        (spec3(), "N=3", P),
    ] {
        let map = lane_map(&s, base).expect("lane map");
        let bits = map.inner[0].k as usize + super::tables::SaTables::new(&s, &map).kx;
        for tw in PAD_RUNS {
            for ia in 1..bits {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples)
                    .unwrap_or_else(|e| panic!("{name} {tw} s {ia}: {e}"));
                assert!(ev.facts.nested_validated);
                for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                    let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                    assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                }
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / 4096.0).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} s {ia}: harness {} vs ledger {want} (tol {tol})",
                    ev.toffoli
                );
            }
        }
    }
}

/// `.p` keeps every inner count and the rounding error, feeds the padded spec's padding subcube
/// from one donor, and on that spec the read stage is narrower (fewer live qubits at the read)
/// for no more Toffolis in the copy, against the same bundle without it.
#[test]
fn pad_runs_keeps_counts_and_merges() {
    let mut merged = 0;
    for (s, pad) in [(spec8(), PAD8), (spec7(), PAD7)] {
        for base in ["imchxgrdkyMnFt", "imchxgrdky4zabCAXZJtRNOBWMenFsY"] {
            let mk = |tw: &str, ia: usize| Params {
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..pad
            };
            let (m0, m1) = (
                lane_map(&s, mk(base, 3)).unwrap(),
                lane_map(&s, mk(&format!("{base}.p"), 3)).unwrap(),
            );
            for (t0, t1) in m0.inner.iter().zip(&m1.inner) {
                assert_eq!(t0.counts(6), t1.counts(6));
            }
            assert_eq!(
                m0.rounding_error(&s).unwrap(),
                m1.rounding_error(&s).unwrap()
            );
            // Every table whose largest item can feed both padding buckets feeds them from one donor;
            // the read merges the subcube only when every table does.
            let mut all_fed = true;
            for (t0, t) in m0.inner.iter().zip(&m1.inner).skip(s.n) {
                let cap = 1u64 << t.mu;
                let big = t0.counts(6).into_iter().max().unwrap();
                let (_, fed) = super::padalias::arrange_runs(t0, 6);
                assert_eq!(fed.contains(&(6, 2)), big >= 2 * cap);
                all_fed &= big >= 2 * cap;
                if big >= 2 * cap {
                    assert_eq!(t.alt[6], t.alt[7], "one donor for the padding subcube");
                }
                assert_eq!((t.keep[6], t.keep[7]), (0, 0));
            }
            println!("{base}: every table fed by one donor: {all_fed}");
            for ia in [4usize, 5] {
                let read_peak = |tw: &str| {
                    let p = mk(tw, ia);
                    let (_, ops, led) = build(&s, p);
                    let stage = led
                        .rows
                        .iter()
                        .position(|r| r.0.contains("inner alias QROAM read"))
                        .unwrap();
                    let (lo, hi) = (
                        if stage == 0 { 0 } else { led.ends[stage - 1] },
                        led.ends[stage],
                    );
                    let mut live = std::collections::BTreeSet::new();
                    let mut peak = 0usize;
                    for (i, op) in ops[..hi].iter().enumerate() {
                        match op.kind {
                            K::Segment | K::Register | K::AppendToRegister | K::DebugPrint => {}
                            K::R | K::Hmr if op.c_condition == NONE => {
                                live.remove(&op.q_target);
                            }
                            _ => {
                                for q in op.qubits() {
                                    live.insert(q);
                                }
                            }
                        }
                        if i >= lo {
                            peak = peak.max(live.len());
                        }
                    }
                    (peak, led.sum("copy").0)
                };
                let (q0, c0) = read_peak(base);
                let (q1, c1) = read_peak(&format!("{base}.p"));
                if all_fed {
                    assert!(q1 < q0, "{base} ia {ia}: read peak {q1} vs {q0}");
                    assert!(c1 <= c0, "{base} ia {ia}: copy {c1} vs {c0}");
                    merged += 1;
                } else {
                    assert!(q1 <= q0, "{base} ia {ia}: read peak {q1} vs {q0}");
                }
            }
        }
    }
    assert!(merged >= 4, "the fed spec merges at every case");
}

/// `.p`'s mutants are rejected by the harness: a subcube merged although its entries differ
/// (paired 113), a collapse `CZ` skipped (45). (The hosting faults 43 / 44 are the gadget's,
/// `paired::tests::pruned_paired_mutants_break_a_lane`.)
#[test]
fn pad_runs_mutants_are_rejected() {
    for (tw, fault) in [
        ("imchxgrdkyMnFt.p", 113u8),
        ("imchxgrdky4zabCAXZJtRNOBWMenFsY.p", 113),
        ("imchxgrdkyMnFt.p", 45),
    ] {
        let caught = [
            (spec8(), PAD8),
            (spec7(), PAD7),
            (spec6(), WIDE6),
            (spec4(), WIDE),
        ]
        .into_iter()
        .any(|(s, base)| {
            (1..5usize).any(|ia| {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                super::paired::FAULT.with(|c| c.set(fault));
                let r = std::panic::catch_unwind(|| build(&s, p));
                super::paired::FAULT.with(|c| c.set(0));
                match r {
                    Err(_) => false,
                    Ok((lm, ops, _)) => eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                }
            })
        });
        assert!(caught, "{tw}: fault {fault} must be rejected");
    }
}

#[test]
fn pad_runs_letter_parses() {
    assert!(Tweaks::parse("imchxgrdky4zabCAXZJtRNOBWMenFsY.p").pad_runs);
    assert!(!Tweaks::parse("imchxgrdky4zabCAXZJtRNOBWMenFsY").pad_runs);
    // A `p` before the dot is lever `p` (the padded QROAM read), not `.p`.
    let t = Tweaks::parse("imchxp");
    assert!(t.pad_share && !t.pad_runs);
    let u = Tweaks::parse("imchx.p");
    assert!(!u.pad_share && u.pad_runs);
    assert!(!Tweaks::parse("all").pad_runs);
    // The extension does not disturb `q<b>_<mode>` or the group digit.
    let v = Tweaks::parse("imchxgrdky4zabq14_1.p");
    assert_eq!(
        (v.hot_groups, v.rank_hold, v.rank_mode, v.pad_runs),
        (4, 14, 1, true)
    );
}

// ---- levers `.g` (graft) and `.h<k>` (Majorana cap) ---------------------------------------------

/// An exact spec whose class layout has spare cells: `B = 5` (square rows of 5 under 3-bit inner
/// indices) and `N = 10` one-body eigenvectors (row 3 of 8, row 4 of 2). At `G = 4` lever `C`'s
/// first class holds the three square rows and the 8-long one-body row (width 8), so groups 0-2
/// have spare cells 6..8, and row 4 forms a trailing class that lever `.g` grafts there.
fn spec9() -> SaSpec {
    let bytes = payload(
        10,
        (3, 5, 1),
        &[8.0, -4.0, 2.0, 6.0, -2.0, 4.0, 1.0, -8.0, 3.0, 5.0],
        &[4.0, -3.0, 2.0],
        &[
            2.0, -1.0, 3.0, 1.0, -2.0, // q0
            1.0, 2.0, -2.0, 4.0, 1.0, // q1
            -3.0, 1.0, 1.0, 2.0, 2.0, // q2
        ],
    );
    parse_payload("test-sa9-v1", &bytes).unwrap()
}

/// [`WIDE`] with a 4-bit outer index for [`spec9`]'s 13 outer items.
const WIDE9: Params = Params {
    outer: (4, 20),
    inner: (3, 20),
    ..WIDE
};

/// Fourteen orbitals and fifteen five-item square rows give four classes of four rows once the
/// first one-body row is included, plus a six-leaf tail for the factored erasure gadget.
fn spec_factor() -> SaSpec {
    let e: Vec<f64> = (0..14)
        .map(|j| if j % 2 == 0 { 2.0 } else { -1.0 })
        .collect();
    let wb = vec![1.0; 15];
    let w: Vec<f64> = (0..75)
        .map(|j| if j % 5 == 0 { 2.0 } else { 1.0 })
        .collect();
    parse_payload("test-sa-factor-v1", &payload(14, (15, 5, 1), &e, &wb, &w)).unwrap()
}

const FACTOR: Params = Params {
    outer: (5, 22),
    inner: (3, 22),
    ..WIDE9
};

#[test]
fn factored_layout_on_small_spec() {
    let s = spec_factor();
    let map = lane_map(&s, FACTOR).unwrap();
    let t = super::tables::SaTables::with_items(&s, &map, true);
    onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = Some(vec![0, 2]));
    let plan = onehot::class_plan_g(&t, 4, false, true, true);
    onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = None);
    assert_eq!(t.rows(), 17);
    assert_eq!(plan.defs.len(), 4, "{plan:?}");
    assert_eq!(plan.grafts.len(), 3, "{plan:?}");
    assert!(
        plan.defs
            .iter()
            .enumerate()
            .all(|(i, d)| { d.members == (4 * i..4 * i + 4).collect::<Vec<_>>() }),
        "{plan:?}"
    );
}

#[test]
fn factored_erasure_passes_exact_spec_and_mutant_fails() {
    let s = spec_factor();
    let p = Params {
        tw: Tweaks::parse("imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4c"),
        inner_a: 2,
        outer_a: 2,
        ..FACTOR
    };
    let run = |fault| {
        onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = Some(vec![0, 2]));
        onehot::FAULT.with(|c| c.set(fault));
        let built = build(&s, p);
        onehot::FAULT.with(|c| c.set(0));
        onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = None);
        built
    };
    let (lm, ops, _) = run(0);
    let ev = eval(&s, p, &lm, &ops, 1 << 12).unwrap();
    assert!(ev.facts.nested_validated);
    for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
        let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
        assert_eq!(v.status, AxisStatus::Verified, "{axis}");
    }
    let (lm_bad, ops_bad, _) = run(220);
    assert!(eval(&s, p, &lm_bad, &ops_bad, 1 << 12).is_err());
}

/// Lever `.g` (and `.h<k>`) bundles: the Li candidate and smaller compositions.
const GRAFT: [&str; 6] = [
    "imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4",
    "imchxgrdky4zabCAXZJtRNOBWMenFsY.g",
    "imchxgrdky4zabCAXZJtRNOBWMnFsYU.gh2",
    "imchxgrdky3zabCAXJBMnFsU.g",
    "imchxgrdky4zabCAXZJBMnFsYUt.gh1",
    "imchxgrdky2zabCXJBMnFt.g",
];

#[test]
fn graft_plans_on_the_spare_cell_spec() {
    let s = spec9();
    let map = lane_map(&s, WIDE9).unwrap();
    let t = super::tables::SaTables::with_items(&s, &map, true);
    let p0 = onehot::class_plan(&t, 4, false, true);
    let p1 = onehot::class_plan_g(&t, 4, false, true, true);
    assert!(!p1.grafts.is_empty(), "spec9 G = 4 grafts");
    assert!(
        p1.e1 < p0.e1,
        "grafting saves slots: {} vs {}",
        p1.e1,
        p0.e1
    );
    // Every leaf has exactly one cell, and grafted leaves sit past their host member.
    let (fold, e1) = onehot::class_layout_g(&t, 4, false, true, true);
    assert_eq!(e1, p1.e1);
    let mut seen = std::collections::BTreeSet::new();
    for (e, &(slot, g, _)) in fold.place.iter().enumerate() {
        assert!(seen.insert((slot, g)), "leaf {e} shares a cell");
        assert_eq!(fold.leaf_at[g][slot], Some(e));
    }
}

/// Every `.g` bundle passes `evaluate` end to end on the spare-cell spec (grafted) and the other
/// exact specs (no graft fits: the plan is lever `C`'s), at every slot width, with verified axes
/// and the harness's `C_step` equal to the ledger.
#[test]
fn graft_passes_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base) in [
        (spec9(), "spare cells", WIDE9),
        (spec8(), "padded, fed", PAD8),
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
    ] {
        let map = lane_map(&s, base).expect("lane map");
        let bits = map.inner[0].k as usize + super::tables::SaTables::new(&s, &map).kx;
        for tw in GRAFT {
            for ia in 1..bits {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let r = std::panic::catch_unwind(|| build(&s, p));
                let Ok((lm, ops, led)) = r else {
                    panic!("{name} {tw} s {ia}: build panicked");
                };
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples)
                    .unwrap_or_else(|e| panic!("{name} {tw} s {ia}: {e}"));
                assert!(ev.facts.nested_validated);
                for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                    let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                    assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                }
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / 4096.0).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} s {ia}: harness {} vs ledger {want} (tol {tol})",
                    ev.toffoli
                );
            }
        }
    }
}

/// The graft's and the Majorana cap's mutants are rejected by the harness on the spare-cell
/// spec: a `lo` move's lowest bit skipped (120), the graft flag's measured fixup skipped (121), its
/// recompute at the erasure skipped (122), the checkpoint's fields without the move (123), the
/// cap's reload skipped (124).
#[test]
fn graft_mutants_are_rejected() {
    let s = spec9();
    for (tw, fault) in [
        ("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 120u8),
        ("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 121),
        ("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 122),
        ("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 123),
        ("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4", 124),
    ] {
        let caught = [None, Some(vec![0usize, 2])].into_iter().any(|force| {
            (1..6usize).any(|ia| {
                let p = Params {
                    outer_a: 1,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..WIDE9
                };
                onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = force.clone());
                onehot::FAULT.with(|c| c.set(fault));
                let r = std::panic::catch_unwind(|| build(&s, p));
                onehot::FAULT.with(|c| c.set(0));
                onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = None);
                match r {
                    Err(_) => true,
                    Ok((lm, ops, _)) => eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                }
            })
        });
        assert!(caught, "{tw}: fault {fault} must be rejected");
    }
}

#[test]
fn graft_and_cap_letters_parse() {
    let t = Tweaks::parse("imchxgrdky4zabCAXZJtRNOBWMnFsYU.pgh4");
    assert!(t.pad_runs && t.graft && t.hold_maj);
    assert_eq!(t.maj_drop, 4);
    assert_eq!(t.hot_groups, 4);
    let u = Tweaks::parse("imchxgrdky4zabCAXZJtRNOBWMnFsYU");
    assert!(!u.graft && u.maj_drop == 0);
    assert_eq!(Tweaks::parse("imchxU.h12").maj_drop, 12);
}

/// Lever `.g` with a moved graft (forced one-body depths `[0, 2]` on the spare-cell spec: row 3
/// whole in the square class, which is then 8 wide, and row 4's sub-row grafted at slot 6 of
/// group 0, `lo ^= 6`): every bundle passes end to end at every slot width.
#[test]
fn graft_with_a_lo_move_passes() {
    let s = spec9();
    let map = lane_map(&s, WIDE9).unwrap();
    let t = super::tables::SaTables::with_items(&s, &map, true);
    onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = Some(vec![0, 2]));
    let plan = onehot::class_plan_g(&t, 4, false, true, true);
    assert!(
        plan.grafts.iter().any(|g| g.mask != 0),
        "a graft moves lo: {:?}",
        plan.grafts
    );
    let bits = map.inner[0].k as usize + super::tables::SaTables::new(&s, &map).kx;
    for tw in GRAFT.iter().filter(|tw| tw.contains('4')) {
        for ia in 1..bits {
            let p = Params {
                outer_a: 1,
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..WIDE9
            };
            let (lm, ops, led) = build(&s, p);
            let ev = eval(&s, p, &lm, &ops, 1 << 12)
                .unwrap_or_else(|e| panic!("forced {tw} s {ia}: {e}"));
            let (want, _) = ledger_c_step(&led, &s, p);
            let tol = 6.0 * (gated_var_bound(&ops) / 4096.0).sqrt() + 1e-9;
            assert!((ev.toffoli - want).abs() <= tol, "forced {tw} s {ia}");
        }
    }
    onehot::FORCE_GRAFT_DEPTHS.with(|c| *c.borrow_mut() = None);
}

// ---- lever `-` (the folded, i_0-refined item one-hot read) --------------------------------------

/// Lever `-` bundles: the Li G = 5 point, the G = 4 item one-hot form, the
/// Reiher product bundle, and small compositions with and without `t`, `+`, `K`, `D`.
const ITEM_FOLD: [&str; 8] = [
    "imchxgrdky5zabCHVIXJtRNOBWPs+-",
    "imchxgrdky4zabCAHVIXZJtRNOBWsY+-",
    "imchxgrdky3zabCHKVIDXtNBWs+-",
    "imchxgrdky5zabCHVIXJtRNOBWPs-",
    "imchxgrdkyHVt-",
    "imchxgrdkyHV-",
    "imchxgrdky3zabCHKVIXD-",
    "imchxgrdky3zfabHKV-",
];

/// Every `-` bundle passes `evaluate` end to end on the four exact specs (both block-count pairs,
/// both SELECTs where `y` allows) and on the padded spec with `+`, with verified axes and the
/// harness's `C_step` equal to the ledger.
#[test]
fn item_fold_passes_on_exact_specs() {
    excl_bundles_pass(&ITEM_FOLD);
    let s = spec7();
    for &tw in ITEM_FOLD.iter().filter(|t| t.contains('+')) {
        for (oa, ia) in [(1, 1), (0, 2), (2, 3)] {
            let p = Params {
                outer_a: oa,
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..PAD7
            };
            let (lm, ops, led) = build(&s, p);
            let var = gated_var_bound(&ops);
            let ev = eval(&s, p, &lm, &ops, 1 << 13)
                .unwrap_or_else(|e| panic!("spec7 {tw} oa {oa} ia {ia}: {e}"));
            assert!(ev.facts.nested_validated);
            for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
            }
            let (want, _) = ledger_c_step(&led, &s, p);
            let tol = 6.0 * (var / f64::from(1u32 << 13)).sqrt() + 1e-9;
            assert!(
                (ev.toffoli - want).abs() <= tol,
                "spec7 {tw}: harness {} vs ledger {want} (tol {tol})",
                ev.toffoli
            );
        }
    }
}

/// Lever `-`'s mutants, each rejected by the harness on at least one spec: a folded slot not
/// made (150), group 2's in-place correction skipped (151), the folded slots' coefficients not
/// corrected by the old slot's (152), a single's Toffoli skipped (153), the linear left-overs
/// skipped (154), the cross left-overs skipped (155), the folded slots' measured erasure fixups
/// skipped (156).
#[test]
fn item_fold_mutants_are_rejected() {
    let faults: &[u8] = &[150, 151, 152, 153, 154, 155, 156];
    for &fault in faults {
        let caught = [
            "imchxgrdkyHVt-",
            "imchxgrdky3zabCHKVIXD-",
            "imchxgrdky5zabCHVIXJtRNOBWPs+-",
        ]
        .iter()
        .any(|tw| {
            let specs = [
                (spec6(), WIDE6),
                (spec4(), WIDE),
                (spec3(), P),
                (spec5(), P),
                (spec7(), PAD7),
            ];
            specs.into_iter().any(|(sp, base)| {
                [(1usize, 1usize), (0, 2), (2, 1), (2, 3)]
                    .iter()
                    .any(|&(oa, ia)| {
                        let p = Params {
                            outer_a: oa,
                            inner_a: ia,
                            tw: Tweaks::parse(tw),
                            ..base
                        };
                        onehot::FAULT.with(|c| c.set(fault));
                        let (lm, ops, _) = build(&sp, p);
                        onehot::FAULT.with(|c| c.set(0));
                        eval(&sp, p, &lm, &ops, 1 << 12).is_err()
                    })
            })
        });
        assert!(caught, "lever -: fault {fault} must be rejected");
    }
}

/// Lever `-` parses only from its own letter and changes no table (the lane map is the same with
/// and without it). Its saving shows on the pinned specs' tables, not on these tiny ones
/// (`itemfold::tests::folded_read_costs`, `tests_combo::combo_scan`).
#[test]
fn item_fold_letter_parses() {
    assert!(Tweaks::parse("imchxgrdky5zabCHVIXJtRNOBWPs+-").item_fold);
    assert!(!Tweaks::parse("imchxgrdky5zabCHVIXJtRNOBWPs+").item_fold);
    assert!(!Tweaks::parse("all").item_fold);
    assert!(!Tweaks::parse("none").item_fold);
    // Next to `q<b>_1.<c>` (whose `.` belongs to the q token) the letter is `-`.
    let q = Tweaks::parse("imchxgrdky5zabCHVIXJtRNOBWPs+q14_1-");
    assert!(q.item_fold && q.rank_hold == 14 && q.rank_mode == 1);
    let s = spec7();
    for base in ["imchxgrdkyHVt+", "imchxgrdky5zabCHVIXJtRNOBWPs+"] {
        let p0 = Params {
            tw: Tweaks::parse(base),
            ..PAD7
        };
        let p1 = Params {
            tw: Tweaks::parse(&format!("{base}-")),
            ..PAD7
        };
        assert_eq!(
            lane_map(&s, p0).unwrap().to_bytes(),
            lane_map(&s, p1).unwrap().to_bytes(),
            "{base}: the fold changes no table"
        );
    }
}

// ---- lever `_` (the unchosen index held in the inner index register) ----------------------------

/// `toff::index_swap`, exhaustively at `k_i = 3`: for every inner index `i`, alias value `a` and
/// coin `lt`, with `chosen = lt ? i : a` and `unchosen = lt ? a : i`, it turns the index into the
/// unchosen value, leaves the other registers as they were, and is its own inverse; mutant 162 is
/// caught.
#[test]
fn index_swap_is_exact() {
    use crate::walk::shared::testsim::Sim;
    for fault in [0u8, 162] {
        let mut caught = false;
        for i in 0..8u64 {
            for a in 0..8u64 {
                for lt in [false, true] {
                    let mut b = Builder::new(1);
                    b.declare_uniform(1);
                    let idx = b.alloc_n(3);
                    let ch = b.alloc_n(3);
                    let un = b.alloc_n(3);
                    let l = b.alloc();
                    onehot::FAULT.with(|c| c.set(fault));
                    super::toff::index_swap(&mut b, l, &ch, &un, &idx);
                    let mid = b.ops().len();
                    super::toff::index_swap(&mut b, l, &ch, &un, &idx);
                    onehot::FAULT.with(|c| c.set(0));
                    let (c, u) = if lt { (i, a) } else { (a, i) };
                    let mut sim = Sim::new(&b, 1);
                    for (j, (&x, (&y, &z))) in idx.iter().zip(ch.iter().zip(&un)).enumerate() {
                        sim.set(x, i >> j & 1 == 1);
                        sim.set(y, c >> j & 1 == 1);
                        sim.set(z, u >> j & 1 == 1);
                    }
                    sim.set(l, lt);
                    sim.run(&b.ops()[..mid]);
                    let ok1 = sim.read(&idx) == u && sim.read(&ch) == c && sim.read(&un) == u;
                    sim.run(&b.ops()[mid..]);
                    let ok2 = sim.read(&idx) == i && sim.read(&ch) == c && sim.read(&un) == u;
                    if fault == 0 {
                        assert!(ok1 && ok2, "i {i} a {a} lt {lt}");
                    } else if !(ok1 && ok2) {
                        caught = true;
                    }
                }
            }
        }
        assert!(fault == 0 || caught, "index_swap fault {fault} not caught");
    }
}

/// Lever `_` bundles: with `t` (and `+`, `K`, the fold `-`) on the Li shapes at G = 4 and 5,
/// with `S` instead of `t`, without `R` (the coin held) and with it (recomputed at PREPARE^dagger),
/// on the Reiher product bundle without `D` (whose stash lives in the alt slot on one-body lanes)
/// and small item-one-hot bundles.
const INDEX_HOST: [&str; 9] = [
    "imchxgrdky5zabCHKVIXJtRNOBWPs+-_",
    "imchxgrdky4zabCAHVIXZJtRNOBWsY+-_",
    "imchxgrdky5zabCHVIXJtRNOBWPs_",
    "imchxgrdky3zabCHKVIXtNBWs+_",
    "imchxgrdky5zabCHVIXJSRNOBW_",
    "imchxgrdky3zabCHKVIXt_",
    "imchxgrdkyHVt-_",
    "imchxgrdkyHt_",
    "imchxgrdky3zfabHKtRNOB_",
];

/// Every `_` bundle passes `evaluate` end to end on the four exact specs (and the padded spec7
/// with `+`), with verified axes and the harness's `C_step` equal to the ledger.
#[test]
fn index_host_passes_on_exact_specs() {
    excl_bundles_pass(&INDEX_HOST);
    let s = spec7();
    for &tw in INDEX_HOST.iter().filter(|t| t.contains('+')) {
        for (oa, ia) in [(1, 1), (0, 2), (2, 3)] {
            let p = Params {
                outer_a: oa,
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..PAD7
            };
            let (lm, ops, led) = build(&s, p);
            let var = gated_var_bound(&ops);
            let ev = eval(&s, p, &lm, &ops, 1 << 13)
                .unwrap_or_else(|e| panic!("spec7 {tw} oa {oa} ia {ia}: {e}"));
            assert!(ev.facts.nested_validated);
            let (want, _) = ledger_c_step(&led, &s, p);
            let tol = 6.0 * (var / f64::from(1u32 << 13)).sqrt() + 1e-9;
            assert!(
                (ev.toffoli - want).abs() <= tol,
                "spec7 {tw}: harness {} vs ledger {want} (tol {tol})",
                ev.toffoli
            );
        }
    }
}

/// Lever `_`'s mutants, each rejected by the harness: the alt register not cleared before it is
/// freed (160), the index not restored at PREPARE^dagger (161), the coin-gated XOR skipped (162).
#[test]
fn index_host_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdkyHt_", &[160, 161, 162]),
        ("imchxgrdky5zabCHVIXJtRNOBWPs_", &[160, 161, 162]),
    ]);
}

/// Lever `_` costs `2 k_i` Toffolis per copy and parses only from its letter (and keeps its letter
/// next to a `q<b>_<m>` token written after it).
#[test]
fn index_host_costs_and_parses() {
    assert!(Tweaks::parse("imchxgrdky5zabCHKVIXJtRNOBWPs+-_").index_host);
    assert!(!Tweaks::parse("imchxgrdky5zabCHKVIXJtRNOBWPs+-").index_host);
    assert!(!Tweaks::parse("imchxgrdky5zabCHVIXJtRNOBWPs+q14_1").index_host);
    let q = Tweaks::parse("imchxgrdky5zabCHVIXJtRNOBWPs+_q14_1");
    assert!(q.index_host && q.rank_hold == 14 && q.rank_mode == 1);
    assert!(!Tweaks::parse("all").index_host);
    let s = spec6();
    for base in ["imchxgrdkyHt", "imchxgrdky3zabCHKVIXt"] {
        let copy = |tw: &str| {
            build(
                &s,
                Params {
                    tw: Tweaks::parse(tw),
                    ..WIDE6
                },
            )
            .2
            .expected_sum("copy")
            .0
        };
        let k_i = f64::from(WIDE6.inner.0);
        assert!(
            (copy(&format!("{base}_")) - copy(base) - 2.0 * k_i).abs() < 1e-9,
            "{base}: lever _ costs 2 k_i per copy"
        );
    }
}

/// Levers `-` and `_` composed with the paired lookup (`G` / `M`, `e n F`, `t`), the held-span
/// delivery `q<b>_1`, `U` and the outer block count, end to
/// end on the exact specs with every slot width the index allows.
const KNEE_NOVEL_FRONT: [&str; 8] = [
    "imchxgrdky4zabCAXZJBMnFsYUt_",
    "imchxgrdky4zabCAXZJtRNOBWMenFsY_",
    "imchxgrdky4zabCAXZJBMnFsYUt_q12_1",
    "imchxgrdky4zabCAHVIXZJtRNOBWsY+-_q12_1",
    "imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_",
    "imchxgrdky5zabCHKVIXJtRNOBWPs+-_q12_1",
    "imchxgrdky3zabCHVIXJtRNOBWs+-_q12_1",
    "imchxgrdkyGFt_",
];

#[test]
fn knee_novel_front_bundles_pass_on_exact_specs() {
    excl_bundles_pass(&KNEE_NOVEL_FRONT);
    let samples: u32 = 1 << 12;
    for (s, name, base) in [
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec7(), "padded", PAD7),
    ] {
        let k_i = base.inner.0 as usize;
        for &tw in KNEE_NOVEL_FRONT
            .iter()
            .filter(|t| t.contains('M') || t.contains('G'))
        {
            for ia in 1..=k_i {
                let p = Params {
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples as usize)
                    .unwrap_or_else(|e| panic!("{name} {tw} ia {ia}: {e}"));
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / f64::from(samples)).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} ia {ia}: harness {} vs ledger {want}",
                    ev.toffoli
                );
            }
        }
    }
}

/// Lever `_`'s mutants stay rejected on the paired-lookup bundle.
#[test]
fn knee_novel_front_mutants_are_rejected() {
    excl_mutants_rejected(&[("imchxgrdky4zabCAXZJBMnFsYUt_", &[160, 161, 162])]);
}

// ---- compositions of `.p`, `.g`, `.h<k>` with `-` and `_` -------------------------------------

/// Levers `.p`, `.g`, `.h<k>` composed with `-` (the folded item read) and
/// `_` (the unchosen index held in the index register): the composed Li bundles.
const KNEE_COMPOSED: [&str; 4] = [
    "imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.gh4",
    "imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.h4",
    "imchxgrdky4zabCAXZJtRNOBWMenFsYU_.pgh4",
    "imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.gh2",
];

/// Every composed bundle passes `evaluate` end to end on the spare-cell spec (grafted), the padded
/// specs and the R = 6 / wide N = 4 specs at several block counts, with verified axes and the
/// harness's `C_step` equal to the ledger.
#[test]
fn knee_compositions_pass_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base) in [
        (spec9(), "spare cells", WIDE9),
        (spec8(), "padded, fed", PAD8),
        (spec7(), "padded", PAD7),
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
    ] {
        for tw in KNEE_COMPOSED {
            for (oa, ia) in [(1usize, 1usize), (0, 2), (2, 3)] {
                let p = Params {
                    outer_a: oa,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples)
                    .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia}: {e}"));
                assert!(ev.facts.nested_validated);
                for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                    let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                    assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                }
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / 4096.0).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} oa {oa} ia {ia}: harness {} vs ledger {want}",
                    ev.toffoli
                );
            }
        }
    }
}

// ---- lever `.e` (the item one-hot erasure's paired phase pass) ---------------------------------

const ERASE_PAIRS: [&str; 6] = [
    "imchxgrdkyHt.e",
    "imchxgrdkyH.e",
    "imchxgrdky3zabCHKVIDXtNBWs+.e",
    "imchxgrdky3zabCHKVIXDt+.e",
    "imchxgrdky4zabCAHVIXZJtRNOBWsYU+-_.geh4",
    "imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4",
];

/// Every `.e` bundle passes `evaluate` end to end on six exact specs at three block-count pairs,
/// with verified axes and the harness's `C_step` equal to the ledger, and costs fewer Toffolis
/// in the copy than the same bundle without it on the padded spec.
#[test]
fn erase_pairs_passes_on_exact_specs() {
    let samples = 1 << 12;
    for (s, name, base) in [
        (spec9(), "spare cells", WIDE9),
        (spec8(), "padded, fed", PAD8),
        (spec7(), "padded", PAD7),
        (spec6(), "R=6", WIDE6),
        (spec4(), "N=4 wide", WIDE),
        (spec5(), "N=5", P),
    ] {
        for tw in ERASE_PAIRS {
            for (oa, ia) in [(1usize, 1usize), (0, 2), (2, 3)] {
                let p = Params {
                    outer_a: oa,
                    inner_a: ia,
                    tw: Tweaks::parse(tw),
                    ..base
                };
                let (lm, ops, led) = build(&s, p);
                let var = gated_var_bound(&ops);
                let ev = eval(&s, p, &lm, &ops, samples)
                    .unwrap_or_else(|e| panic!("{name} {tw} oa {oa} ia {ia}: {e}"));
                assert!(ev.facts.nested_validated);
                for axis in ["encoding", "lane_map", "select", "rotation", "uncompute"] {
                    let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
                    assert_eq!(v.status, AxisStatus::Verified, "{tw}: {axis}");
                }
                let (want, _) = ledger_c_step(&led, &s, p);
                let tol = 6.0 * (var / 4096.0).sqrt() + 1e-9;
                assert!(
                    (ev.toffoli - want).abs() <= tol,
                    "{name} {tw} oa {oa} ia {ia}: harness {} vs ledger {want}",
                    ev.toffoli
                );
            }
        }
    }
    let s = spec7();
    for tw in ["imchxgrdkyHt", "imchxgrdky3zabCHKVIDXtNBWs+"] {
        let copy = |t: &str| {
            let p = Params {
                inner_a: 2,
                tw: Tweaks::parse(t),
                ..PAD7
            };
            build(&s, p).2.sum("copy").0
        };
        let (c0, c1) = (copy(tw), copy(&format!("{tw}.e")));
        assert!(c1 < c0, "{tw}: .e {c1} vs {c0}");
    }
}

/// `.e`'s mutants are rejected by the harness: the pair's `CCZ` skipped (130), the classical
/// left-over skipped (131), the final left-over phase skipped (132).
#[test]
fn erase_pairs_mutants_are_rejected() {
    for fault in [130u8, 131, 132] {
        let caught = [
            (spec7(), PAD7),
            (spec6(), WIDE6),
            (spec4(), WIDE),
            (spec8(), PAD8),
        ]
        .into_iter()
        .any(|(s, base)| {
            ["imchxgrdkyHt.e", "imchxgrdky3zabCHKVIDXtNBWs+.e"]
                .iter()
                .any(|tw| {
                    [(1usize, 1usize), (0, 2), (2, 3)].iter().any(|&(oa, ia)| {
                        let p = Params {
                            outer_a: oa,
                            inner_a: ia,
                            tw: Tweaks::parse(tw),
                            ..base
                        };
                        onehot::FAULT.with(|c| c.set(fault));
                        let r = std::panic::catch_unwind(|| build(&s, p));
                        onehot::FAULT.with(|c| c.set(0));
                        match r {
                            Err(_) => true,
                            Ok((lm, ops, _)) => eval(&s, p, &lm, &ops, 1 << 12).is_err(),
                        }
                    })
                })
        });
        assert!(caught, ".e fault {fault} must be rejected");
    }
}

// ---- `q<b>_1.<c>` composed with `.e` -----------------------------------------------------------

/// The composed bundles (the Majorana cut and the paired phase pass in one step), end to end on
/// the exact specs: verified axes, the harness's `C_step` equal to the ledger. The
/// Reiher `kc_*` pins are this shape.
#[test]
fn park_with_erase_pairs_passes_on_exact_specs() {
    excl_bundles_pass(&[
        "imchxgrdky3zabCHKVIDXtNBWs+q13_1.5.e",
        "imchxgrdky3zabCHKVIDXtNBWs+Rq13_1.2.e",
        "imchxgrdky3zabCHKVIDXtNBWsq13_1.64.e",
        "imchxgrdky3zabq11_1.4.e",
        "imchxgrdky4zabq8_1.3.e",
    ]);
}

/// In the composed bundle each gadget's mutants are still rejected on their own: `.e`'s 130-132
/// and the Majorana cut's 91 / 93 (133, the cut not emitted, only moves the peak; it is caught by
/// `rankdel`'s live-row check).
#[test]
fn park_with_erase_pairs_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdky3zabCHKVIDXtNBWs+q13_1.5.e", &[130, 131, 132]),
        ("imchxgrdky3zabCHKVIXDq10_1.2.e", &[91, 93]),
    ]);
}

// ---- lever `~` -------------------------------------------------------------------------------

/// Lever `~` bundles: the base shape (four-group item one-hot `T` with `S`, `_`) and its G = 4 / 3
/// neighbours, the same with two groups (`V I`) and contiguous (`H` alone), with `t` instead of `S`
/// (the rooted re-read), with `+` (the offset re-read part), with the fold `-` (the PREPARE read
/// folded, the re-read not), and Li's G = 6 / 7 shapes (`o`, `t +`).
const HOT_KEEP: [&str; 13] = [
    "imchxgrdky5zabfHKVITXSNOBWs_~",
    "imchxgrdky4zZabfHKVITXSNOBWs_~",
    "imchxgrdky3zabCHKVITXSNOBWs_~",
    "imchxgrdky6zabCHVIXZJNOBWsot+_~",
    "imchxgrdky7zabCHVIXJNOBWsot+_~",
    "imchxgrdky3zabCHKVIXDSNBWUT_~",
    "imchxgrdky5zabCHVIXJSNOBW_~",
    "imchxgrdky3zabCHKVIXt_~",
    "imchxgrdky3zabCHKVIXtNBWs+_~",
    "imchxgrdky5zabCHKVIXJtNOBWPs+-_~",
    "imchxgrdkyHVt-_~",
    "imchxgrdkyHt_~",
    "imchxgrdkyHS_~",
];

/// Every `~` bundle passes `evaluate` end to end on the four exact specs (the wide N = 4 spec has
/// nonzero inner keeps; the dyadic N = 3 / N = 5 specs have keep 0 and would hide a keep-release
/// fault) and the padded spec7 with `+`, with verified axes and the harness's `C_step` equal to the
/// ledger within the gated-sampling tolerance.
#[test]
fn hot_keep_passes_on_exact_specs() {
    // (`D` with `~` is refused: drop it from the four-group bundle.)
    let tws: Vec<String> = HOT_KEEP.iter().map(|t| t.replace('D', "")).collect();
    let tws: Vec<&str> = tws.iter().map(String::as_str).collect();
    excl_bundles_pass(&tws);
    let s = spec7();
    for &tw in tws.iter().filter(|t| t.contains('+')) {
        for (oa, ia) in [(1, 1), (0, 2), (2, 3)] {
            let p = Params {
                outer_a: oa,
                inner_a: ia,
                tw: Tweaks::parse(tw),
                ..PAD7
            };
            let (lm, ops, led) = build(&s, p);
            let var = gated_var_bound(&ops);
            let ev = eval(&s, p, &lm, &ops, 1 << 13)
                .unwrap_or_else(|e| panic!("spec7 {tw} oa {oa} ia {ia}: {e}"));
            assert!(ev.facts.nested_validated);
            let (want, _) = ledger_c_step(&led, &s, p);
            let tol = 6.0 * (var / f64::from(1u32 << 13)).sqrt() + 1e-9;
            assert!(
                (ev.toffoli - want).abs() <= tol,
                "spec7 {tw}: harness {} vs ledger {want} (tol {tol})",
                ev.toffoli
            );
        }
    }
}

/// Lever `~`'s mutants, each rejected by the harness end to end: the comparison's phase dropped
/// (201), the block run on outcome 0 instead of 1 (202), the early keep outcomes left out of the
/// erasure pass (205), the re-read's outcomes left out (206), the re-read run before the item
/// one-hot is rewritten (207).
#[test]
fn hot_keep_mutants_are_rejected() {
    excl_mutants_rejected(&[
        ("imchxgrdky5zabfHKVITXSNOBWs_~", &[201, 202, 205, 206, 207]),
        ("imchxgrdky3zabCHKVIXt_~", &[201, 202, 205, 206, 207]),
        ("imchxgrdkyHS_~", &[201, 202, 205, 206, 207]),
    ]);
}

/// The letter parses only from `~`; `~` with `R`, `D`, `j` or without `_` is refused.
#[test]
fn hot_keep_parses_and_refuses() {
    assert!(Tweaks::parse("imchxgrdky5zabfHKVITXSNOBWs_~").hot_keep_release);
    assert!(!Tweaks::parse("imchxgrdky5zabfHKVITXSNOBWs_").hot_keep_release);
    assert!(!Tweaks::parse("all").hot_keep_release);
    assert!(!Tweaks::parse("none").hot_keep_release);
    let q = Tweaks::parse("imchxgrdky5zabfHKVITXSNOBWs_~q2_1");
    assert!(q.hot_keep_release && q.index_host && q.rank_hold == 2 && q.rank_mode == 1);
    let s = spec4();
    for bad in [
        "imchxgrdky5zabfHKVITXSRNOBWs_~",
        "imchxgrdky3zabCHKVIXDSNBWUT_~",
        "imchxgrdky5zabfHKVITXSNOBWs~",
        "imchxgrdkyHt_j~",
    ] {
        let p = Params {
            tw: Tweaks::parse(bad),
            ..WIDE
        };
        let r = std::panic::catch_unwind(|| build(&s, p));
        assert!(r.is_err(), "{bad} must be refused");
    }
}

/// Self-aliased inner buckets (`alt == own`) keep 0 on both pinned literature specs, so `lt` is 0
/// there and the gated phase is trivial (no special path; the protocol is exact either way).
#[test]
#[ignore = "pinned specs; run in release"]
fn hot_keep_self_buckets_keep_zero() {
    for id in ["reiher-sa-est-v1", "li-sa-est-v1"] {
        let boxed = super::tests_combo::pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        for mu in [8u32, 9] {
            let base = Params::for_spec(s);
            let p = Params {
                outer: (base.outer.0, mu),
                inner: (base.inner.0, mu),
                tw: Tweaks::parse("imchxgrdky5zabfHKVITXSNOBWs_~"),
                ..base
            };
            let map = lane_map(s, p).unwrap();
            let t = super::tables::SaTables::with(s, &map, p.tw.derive_id, p.tw.compact_outer);
            let k_i = t.k_i;
            let (start, limit) = t.item_range();
            let mu_i = p.inner.1 as usize;
            let mut selfs = 0usize;
            for x in start..limit {
                let wd = t.item_inner_word(x);
                let get = |at: usize, w: usize| -> u64 {
                    (0..w).fold(0, |acc, j| {
                        acc | (wd[(at + j) / 64] >> ((at + j) % 64) & 1) << j
                    })
                };
                let keep = get(0, mu_i);
                let fb = t.flag_bits();
                let alt = get(mu_i + fb, k_i);
                if alt == x & ((1 << k_i) - 1) && x & ((1 << k_i) - 1) <= s.b as u64 {
                    selfs += 1;
                    assert_eq!(
                        keep, 0,
                        "{id} mu {mu}: self-aliased bucket {x} keeps {keep}"
                    );
                }
            }
            println!("{id} mu {mu}: {selfs} self-aliased inner buckets, all keep 0");
        }
    }
}

// ---- lever `~` composed with the extension levers ---------------------------------------------

/// The composed `~` bundles: `~` with the paired erasure `.e` (the
/// re-read part goes through `phase_rooted_paired`), with `.eh4` and `U`, with
/// `q<b>_1.<c>` (budget 13, the smallest the exact specs' words admit, as in tests_knee), and the Li-shaped
/// G = 5 / 6 / 7 strings of the full-K pins.
const HOT_KEEP_COMPOSED: [&str; 9] = [
    "imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4",
    "imchxgrdky7zabCHKVIXJOBWsotU+_~.e",
    "imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4",
    "imchxgrdky3zabCHKVIXtNBWs+_~.e",
    "imchxgrdkyHVt-_~.e",
    "imchxgrdkyHVS_~.e",
    "imchxgrdky5zabfHKVITXSNOBWs_~q13_1.5",
    "imchxgrdky4zZabfHVITXSNOBWUs_~q13_1.2",
    "imchxgrdky3zabCHKVIXtNBWs+_~q13_1.5.e",
];

/// Every composed `~` bundle passes `evaluate` end to end on the four exact specs (verified axes,
/// harness `C_step` equal to the ledger within the gated-sampling tolerance).
#[test]
fn hot_keep_composed_passes_on_exact_specs() {
    excl_bundles_pass(&HOT_KEEP_COMPOSED);
}

/// With `.e`, the re-read's outcomes (206), the early keep outcomes (205) and the paired pass's
/// own faults (130: the pair CCZ dropped; 131: the quadratic slot terms dropped) are each rejected
/// end to end, so the paired pass really carries `~`'s third part.
#[test]
fn hot_keep_composed_mutants_are_rejected() {
    excl_mutants_rejected(&[
        (
            "imchxgrdky3zabCHKVIXtNBWs+_~.e",
            &[201, 202, 205, 206, 130, 131],
        ),
        ("imchxgrdkyHVS_~.e", &[201, 202, 205, 206, 130, 131]),
    ]);
}

/// On every composed two-group bundle, `~` with `.e` costs no more ledger Toffolis than the same
/// bundle without `.e` (the paired pass, now carrying the re-read part, never adds work).
#[test]
fn hot_keep_paired_erase_is_no_dearer() {
    let s = spec4();
    for tw in HOT_KEEP_COMPOSED.iter().filter(|t| t.ends_with(".e")) {
        let base = tw.trim_end_matches(".e");
        let c = |t: &str| {
            let p = Params {
                tw: Tweaks::parse(t),
                ..WIDE
            };
            let (_, _, led) = build(&s, p);
            ledger_c_step(&led, &s, p).0
        };
        assert!(c(tw) <= c(base) + 1e-9, "{tw}: {} > {}", c(tw), c(base));
    }
}
