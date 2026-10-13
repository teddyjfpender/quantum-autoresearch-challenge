//! Adversarial test: memory that a small submission can force in the outcome-coverage mode's
//! static pass (`sim::condition::analyze`, which runs before any lane; see also
//! tests/adversarial_dos.rs). A test binary of its own because it replaces the global allocator
//! with a counting one, and it holds one test so that nothing else allocates beside it.
//!
//! **Fixed: affine rows without a size limit.** The pass followed every classical bit as an
//! affine form of the outcomes with no limit on the form's length or on the total held, only a
//! work cap of `2^31` entries touched. A parity of 40,000 outcomes copied into 8,000 bits
//! (96,000 ops, 1.9 MB of `ops.bin`) made it hold 8,000 rows of 40,000 variables: 1.2 GiB at
//! the peak, linear in the work spent, so several GiB at the cap. An equation is now `Opaque`
//! past `condition::MAX_ROW` variables and the pass holds at most a fixed budget of entries
//! (`HELD_CAP`); the same stream peaks below 16 MiB in the pass.
mod harness_common;

use femoco_walk::circuit::{Op, OperationType as K, NONE};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{self, score_json, Inputs, Options, OutcomeCoverage};
use femoco_walk::sim::condition::{self, Class};
use femoco_walk::sim::validate;
use femoco_walk::sim::{compile, Layout};
use femoco_walk::spec::EncodingSpec;
use harness_common::{alias, build, declared_only, family, ops_file, Mutation, TestSpec, N};
use std::alloc::{GlobalAlloc, Layout as MemLayout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded unchanged to the system allocator; the counters are
// bookkeeping only.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: MemLayout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: MemLayout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        System.dealloc(p, l);
    }
    unsafe fn realloc(&self, p: *mut u8, l: MemLayout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add(new - l.size(), Ordering::Relaxed) + new - l.size();
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Heap bytes `f` adds at its peak to what is live when it starts.
fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let out = f();
    (out, PEAK.load(Ordering::Relaxed).saturating_sub(before))
}

const MIB: usize = 1 << 20;

/// The toy circuit, then `outcomes` measurements, one bit holding their parity, and `copies`
/// bits each set to that parity and inverted once more (so each holds an equation of its own).
/// Every conditioned op runs on half the lanes, so the stream is an honest circuit as far as
/// the lanes go.
fn long_parities(outcomes: u32, copies: u32) -> Vec<Op> {
    let (q0, b0) = (4000u32, 4000u32);
    let mut ops = build(&TestSpec::new(), &alias(), Mutation::None);
    let invert = |bit: u32, cond: u32| {
        let mut o = Op::new(K::BitInvert);
        (o.c_target, o.c_condition) = (bit, cond);
        o
    };
    for i in 0..outcomes {
        let mut h = Op::new(K::Hmr);
        (h.q_target, h.c_target) = (q0, b0 + i);
        ops.push(h);
    }
    let parity = b0 + outcomes;
    ops.extend((0..outcomes).map(|i| invert(parity, b0 + i)));
    for k in 0..copies {
        ops.extend([invert(parity + 1 + k, parity), invert(parity + 1 + k, NONE)]);
    }
    ops
}

#[test]
fn long_parities_do_not_multiply_the_static_pass_memory() {
    let ops = long_parities(40_000, 8_000);
    let layout = Layout {
        system: N,
        uniform: alias().uniform_bits(),
    };
    let compiled = compile(&ops, &layout, None).unwrap();
    let (analysis, peak) = peak_of(|| condition::analyze(&compiled.ops));
    println!(
        "static pass: {} sites, peak heap {:.1} MiB, held {} units",
        analysis.sites.len(),
        peak as f64 / MIB as f64,
        analysis.peak_held
    );
    // Before the fix: 1,226 MiB.
    assert!(peak < 16 * MIB, "{} MiB", peak / MIB);
    assert!(analysis.memory_capped && !analysis.capped);
    // The parity bit is not followed past the row limit, so the sites on it are opaque, never
    // proved unreachable.
    let last = analysis.sites.last().unwrap();
    assert_eq!(last.class, Class::Opaque);
    assert!(analysis.sites.iter().all(|s| s.class != Class::Dead));

    // The whole evaluation in the mode: it passes (every site runs on half the lanes) and
    // reports that the cap was reached.
    let spec = TestSpec::new();
    let file = ops_file(&ops);
    let lm = alias().to_bytes();
    let fam = family(spec.id());
    let inp = Inputs {
        spec: &spec,
        lanemap: &lm,
        family: &fam,
        ops: &file,
        samples: 2048,
        tracker: None,
        check: &declared_only,
    };
    let opts = Options {
        outcome_coverage: OutcomeCoverage::Full,
        ..Options::default()
    };
    let (ev, peak) = peak_of(|| score::evaluate_opts(&inp, validate::reference, None, opts));
    let ev = ev.unwrap();
    println!("evaluation: peak heap {:.1} MiB", peak as f64 / MIB as f64);
    assert!(peak < 128 * MIB, "{} MiB", peak / MIB);
    let j = score_json(&ev);
    let c = &j["metrics"]["outcome_coverage"];
    assert_eq!(c["static"]["memory_cap_reached"], true);
    assert_eq!(c["static"]["work_cap_reached"], false);
    assert_eq!(c["directed"]["strata"], 0);
    assert!(c["static"]["opaque"].as_u64().unwrap() >= 8_000);
}
