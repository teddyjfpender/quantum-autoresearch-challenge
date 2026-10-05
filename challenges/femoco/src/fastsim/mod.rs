//! The fast lane engine (`eval_circuit --engine sliced`; spec/FAST-EVALUATOR.md). Tooling, not
//! yet adopted: the reference engine (`crate::sim::validate`) stays the default and is the
//! oracle this engine must equal on every observable.
//!
//! It replaces exactly `validate::run_with`: same `Context`, same lanes, same `Outcome`. Per
//! 512-lane batch (the reference's batch, with the reference's outcome stream):
//!
//! 1. `exec` runs the batch bit-sliced and records, per lane, the exact sequence of tracker
//!    calls the reference would make (it runs no tracker).
//! 2. The end-of-lane checks run in the reference's order. A lane with a tracker (or a
//!    non-monomial reference) is judged by the reference `finish` verdict on its *trace key*:
//!    every tracker call, the final frame with its phase, and the reference operator. The
//!    verdict is a pure function of that key, so it is computed once per distinct key
//!    (`Memo`; full keys are compared, so a hash collision cannot change a verdict) by
//!    `gauss::verdict` (bit-identical kernels, or the reference tracker itself).
//! 3. A batch in which anything fails (an execution error, any lane's check) is re-run by the
//!    reference's own `run_batch`, whose `Outcome` (failure counts, first failures, messages,
//!    partial tallies) replaces the fast one. A passing batch's `Outcome` is its lane count and
//!    tally, which equal the reference's because the executor's tallies do.
//!
//! `Outcome::merge` is order-independent, so the merged result is the reference's.
pub mod exec;
pub mod fold;
pub mod gauss;
pub mod kernels;
pub mod pfblock;
pub mod pfreal;
pub mod prof;
#[cfg(test)]
mod tests;

use crate::fiat_shamir::{hmr_stream, Lane};
use crate::sim::jw::monomial_to_frame;
use crate::sim::lanes::{LANES, W};
use crate::sim::tracker::LaneFrame;
use crate::sim::validate::{self, check_identity, compare, Context, Nested, Outcome};
use crate::spec::SystemOp;
use exec::{put_frame, put_var, Exec, TAG_CALL_FRAME, TAG_CALL_ID};
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Which lane engine evaluates a submission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// `crate::sim::validate` (the default and the oracle).
    #[default]
    Reference,
    /// This module.
    Sliced,
}

impl Engine {
    /// # Errors
    /// An unknown name.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "reference" => Ok(Self::Reference),
            "sliced" => Ok(Self::Sliced),
            _ => Err(format!("unknown engine {s} (reference, sliced)")),
        }
    }

    /// This engine as a `validate::Engine` (what `score::evaluate_with` takes).
    #[must_use]
    pub fn lane_engine(self) -> validate::Engine {
        match self {
            Self::Reference => validate::reference,
            Self::Sliced => run_probed,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Sliced => "sliced",
        }
    }
}

/// Counters for the notes (stderr with `FEMOCO_FASTSIM_STATS=1`; never in `score.json`).
#[derive(Default)]
pub struct Stats {
    pub batches: AtomicU64,
    pub fallback_batches: AtomicU64,
    pub keyed_lanes: AtomicU64,
    pub distinct_keys: AtomicU64,
    pub key_bytes: AtomicU64,
    /// The guarded fold's decisions (`fold::GuardCounts`).
    pub guard: fold::GuardCounts,
}

struct Entry {
    verdict: OnceLock<bool>,
    /// Keeps alive every `Arc<Network>` whose address the key holds (`put_reference`), so no
    /// other network can take that address while the memo exists.
    _pin: Option<SystemOp>,
}

const SHARDS: usize = 64;

type Shard = HashMap<Box<[u8]>, Arc<Entry>>;

/// Compute-once map from trace key to the reference verdict.
struct Memo {
    shards: Vec<Mutex<Shard>>,
    hasher: std::hash::RandomState,
}

impl Memo {
    fn new() -> Self {
        Self {
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            hasher: std::hash::RandomState::new(),
        }
    }

    fn get(
        &self,
        key: &[u8],
        pin: Option<&SystemOp>,
        stats: &Stats,
        compute: impl FnOnce() -> bool,
    ) -> bool {
        let mut h = self.hasher.build_hasher();
        h.write(key);
        let shard = (h.finish() as usize) % SHARDS;
        let entry = {
            let Ok(mut map) = self.shards[shard].lock() else {
                return compute();
            };
            if let Some(e) = map.get(key) {
                Arc::clone(e)
            } else {
                stats.distinct_keys.fetch_add(1, Ordering::Relaxed);
                stats
                    .key_bytes
                    .fetch_add(key.len() as u64, Ordering::Relaxed);
                let e = Arc::new(Entry {
                    verdict: OnceLock::new(),
                    _pin: pin.cloned(),
                });
                map.insert(key.into(), Arc::clone(&e));
                e
            }
        };
        *entry.verdict.get_or_init(compute)
    }
}

/// The reference operator's canonical bytes. A `Rotated` part's network is named by its `Arc`
/// address (the memo entry pins it), so equal bytes mean the same operator.
fn put_reference(out: &mut Vec<u8>, r: Option<&SystemOp>) {
    match r {
        None => out.push(0),
        Some(SystemOp::Monomial(m)) => {
            out.push(1);
            out.push(m.phase);
            put_var(out, m.majoranas.len() as u64);
            for &a in &m.majoranas {
                put_var(out, u64::from(a));
            }
        }
        Some(SystemOp::Rotated(r)) => {
            out.push(2);
            out.push(r.phase);
            put_var(out, r.parts.len() as u64);
            for p in &r.parts {
                out.extend_from_slice(&(Arc::as_ptr(&p.network) as usize as u64).to_le_bytes());
                out.push(p.spin.map_or(0xff, |s| s));
                put_var(out, p.majoranas.len() as u64);
                for &a in &p.majoranas {
                    put_var(out, u64::from(a));
                }
            }
        }
    }
}

fn get_var(b: &[u8], at: &mut usize) -> u64 {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let x = b[*at];
        *at += 1;
        v |= u64::from(x & 0x7f) << shift;
        if x & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

/// Decodes a lane trace (`exec`'s records) into the tracker calls it stands for.
fn decode_calls(t: &[u8], n: usize) -> Vec<(Option<LaneFrame>, usize, usize, u64)> {
    let words = n.div_ceil(64);
    let mut out = Vec::new();
    let mut at = 0;
    while at < t.len() {
        let tag = t[at];
        at += 1;
        let p = get_var(t, &mut at) as usize;
        let q = get_var(t, &mut at) as usize;
        let angle = get_var(t, &mut at);
        if tag != TAG_CALL_FRAME {
            debug_assert_eq!(tag, TAG_CALL_ID);
            out.push((None, p, q, angle));
            continue;
        }
        let mut f = LaneFrame {
            x: vec![0; words],
            z: vec![0; words],
            s_pow: vec![0; n],
            phase: 0,
        };
        let word = |at: &mut usize| {
            let v = u64::from_le_bytes(t[*at..*at + 8].try_into().unwrap_or([0; 8]));
            *at += 8;
            v
        };
        for i in 0..words {
            f.x[i] = word(&mut at);
        }
        for i in 0..words {
            f.z[i] = word(&mut at);
        }
        for c in 0..n.div_ceil(4) {
            let b = t[at];
            at += 1;
            for i in 0..4 {
                if 4 * c + i < n {
                    f.s_pow[4 * c + i] = (b >> (2 * i)) & 3;
                }
            }
        }
        out.push((Some(f), p, q, angle));
    }
    out
}

fn get<const WW: usize>(v: &[u64], q: u32, lane: usize) -> bool {
    v[q as usize * WW + lane / 64] >> (lane % 64) & 1 == 1
}

/// Batches per pass (`Exec<WW>` with `WW = PASS * W`): enough passes for every thread, and
/// planes small enough to stay in cache (a performance choice only; results do not depend on
/// it, `tests` run several widths).
fn pass_batches(ctx: &Context<'_>, batches: usize) -> usize {
    let threads = rayon::current_num_threads().max(1);
    let planes = ctx.compiled.num_qubits as usize + ctx.compiled.num_bits as usize;
    let mut b = 8;
    while b > 1 && (batches < 2 * threads * b || planes * b * W * 8 > 8 << 20) {
        b /= 2;
    }
    b
}

/// Runs every lane; equal to `validate::run_with` on every observable (module docs).
#[must_use]
pub fn run(ctx: &Context<'_>, nested: Option<&Nested<'_>>, lanes: &[Lane]) -> Outcome {
    run_probed(ctx, nested, lanes, None)
}

/// [`run`] as a `validate::Engine` (the equivalence harness, `crate::equiv`): when `verdicts` is
/// given (one byte per lane, sample order) it also writes each lane's verdict. A batch the fast
/// path passes is all `VERDICT_PASS` (every lane's checks passed); a batch it does not pass is
/// re-run by the reference's `run_batch`, which writes the reference's own verdicts. Recording
/// verdicts changes nothing else.
#[must_use]
pub fn run_probed(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    lanes: &[Lane],
    verdicts: Option<&mut [u8]>,
) -> Outcome {
    let width = std::env::var("FEMOCO_FASTSIM_PASS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| [1, 2, 4, 8].contains(v))
        .unwrap_or_else(|| pass_batches(ctx, lanes.len().div_ceil(LANES)));
    run_width(ctx, nested, lanes, width, verdicts)
}

/// `run_probed` with `width` batches per pass (1, 2, 4 or 8).
#[must_use]
pub fn run_width(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    lanes: &[Lane],
    width: usize,
    verdicts: Option<&mut [u8]>,
) -> Outcome {
    let memo = Memo::new();
    let stats = Stats::default();
    let pass = |pi: usize, pass: &[Lane], v: Option<&mut [u8]>| {
        let first = pi * width;
        let chunks: Vec<&[Lane]> = pass.chunks(LANES).collect();
        stats
            .batches
            .fetch_add(chunks.len() as u64, Ordering::Relaxed);
        let wide = match width {
            8 => fast_pass::<64>(ctx, nested, first, &chunks, &memo, &stats),
            4 => fast_pass::<32>(ctx, nested, first, &chunks, &memo, &stats),
            2 => fast_pass::<16>(ctx, nested, first, &chunks, &memo, &stats),
            _ => None,
        };
        if let Some(o) = wide {
            if let Some(v) = v {
                v.fill(validate::VERDICT_PASS);
            }
            return o;
        }
        // Batch by batch; a batch that still fails is re-run on the reference.
        let mut vs: Vec<Option<&mut [u8]>> = match v {
            Some(v) => v.chunks_mut(LANES).map(Some).collect(),
            None => chunks.iter().map(|_| None).collect(),
        };
        chunks
            .iter()
            .zip(vs.iter_mut())
            .enumerate()
            .map(|(i, (chunk, v))| {
                let b = first + i;
                match fast_pass::<W>(ctx, nested, b, &[chunk], &memo, &stats) {
                    Some(o) => {
                        if let Some(v) = v.as_deref_mut() {
                            v.fill(validate::VERDICT_PASS);
                        }
                        o
                    }
                    None => {
                        stats.fallback_batches.fetch_add(1, Ordering::Relaxed);
                        validate::run_batch(ctx, nested, b, chunk, v.as_deref_mut())
                    }
                }
            })
            .fold(Outcome::default(), Outcome::merge)
    };
    let out = match verdicts {
        None => lanes
            .par_chunks(LANES * width)
            .enumerate()
            .map(|(pi, p)| pass(pi, p, None))
            .reduce(Outcome::default, Outcome::merge),
        Some(v) => {
            assert_eq!(v.len(), lanes.len(), "one verdict per lane");
            lanes
                .par_chunks(LANES * width)
                .zip(v.par_chunks_mut(LANES * width))
                .enumerate()
                .map(|(pi, (p, v))| pass(pi, p, Some(v)))
                .reduce(Outcome::default, Outcome::merge)
        }
    };
    prof::report();
    let snapshot = stats.snapshot();
    if let Ok(mut last) = LAST_STATS.lock() {
        *last = snapshot.clone();
    }
    let g = &stats.guard;
    let (cert, fb, small) = (
        g.certified.load(Ordering::Relaxed),
        g.fallback.load(Ordering::Relaxed),
        g.small.load(Ordering::Relaxed),
    );
    let max_g = f64::from_bits(g.max_g.load(Ordering::Relaxed));
    let ratio = fold::take_ratio();
    if std::env::var_os("FEMOCO_FASTSIM_STATS").is_some() {
        eprintln!(
            "fastsim fold: {cert} residuals certified by the guard, {fb} guard fallbacks to the exact residual, {small} exact (too few vectors to fold); max g {max_g:.3e}, max |off_ref - off_fold| / g {ratio:.3e}"
        );
        eprintln!(
            "fastsim: {} batches ({} re-run on the reference), {} tracked lanes, {} distinct keys ({:.1} MB of keys), {width} batches per pass",
            stats.batches.load(Ordering::Relaxed),
            stats.fallback_batches.load(Ordering::Relaxed),
            stats.keyed_lanes.load(Ordering::Relaxed),
            stats.distinct_keys.load(Ordering::Relaxed),
            stats.key_bytes.load(Ordering::Relaxed) as f64 / 1e6,
        );
    }
    out
}

/// The counters of the most recent [`run`] in this process (any thread), by name, for the
/// equivalence harness's reports (`crate::equiv::engine_counters`). Never in `score.json`.
static LAST_STATS: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

/// The counters of the most recent [`run`] in this process, by name (empty before any run).
#[must_use]
pub fn last_stats() -> Vec<(&'static str, u64)> {
    LAST_STATS.lock().map(|v| v.clone()).unwrap_or_default()
}

impl Stats {
    /// Every counter by name. A counter added to `Stats` belongs here too, so the harness
    /// reports it (a guard-band fallback counter must be named with `guard` in it: the harness's
    /// near-threshold test looks for it).
    fn snapshot(&self) -> Vec<(&'static str, u64)> {
        vec![
            ("batches", self.batches.load(Ordering::Relaxed)),
            (
                "fallback_batches",
                self.fallback_batches.load(Ordering::Relaxed),
            ),
            ("keyed_lanes", self.keyed_lanes.load(Ordering::Relaxed)),
            ("distinct_keys", self.distinct_keys.load(Ordering::Relaxed)),
            ("key_bytes", self.key_bytes.load(Ordering::Relaxed)),
            (
                "fold_certified",
                self.guard.certified.load(Ordering::Relaxed),
            ),
            (
                "guard_fallbacks",
                self.guard.fallback.load(Ordering::Relaxed),
            ),
            (
                "guard_exact_small",
                self.guard.small.load(Ordering::Relaxed),
            ),
            // The band's width: the largest g, in units of 1e-18, rounded up (no `guard` in the
            // name: it is not a fallback count).
            ("fold_max_g_attos", {
                let g = f64::from_bits(self.guard.max_g.load(Ordering::Relaxed));
                if g.is_finite() {
                    (g * 1e18).ceil() as u64
                } else {
                    u64::MAX
                }
            }),
        ]
    }
}

/// One pass of `WW / W` consecutive batches starting at batch `first`; `None` when any of them
/// must be re-run (the caller then runs them one by one).
fn fast_pass<const WW: usize>(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    first: usize,
    chunks: &[&[Lane]],
    memo: &Memo,
    stats: &Stats,
) -> Option<Outcome> {
    let l = &ctx.layout;
    let per = WW / W;
    if chunks.is_empty() || chunks.len() > per {
        return None;
    }
    let mut ex = Exec::<WW>::new(ctx.compiled, l.system, ctx.tracker);
    if let Some(n) = nested {
        let rl: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(i, c)| validate::reflect_lanes(l, n, (first + i) * LANES, c))
            .collect();
        ex.reflect = Some(exec::WideReflect::new::<WW>(&rl));
    }
    for (bi, chunk) in chunks.iter().enumerate() {
        for (i, lane) in chunk.iter().enumerate() {
            let at = bi * LANES + i;
            ex.active[at / 64] |= 1 << (at % 64);
            let mut set = |q: u32| ex.q[q as usize * WW + at / 64] |= 1 << (at % 64);
            if lane.c {
                set(0);
            }
            for k in 0..l.uniform {
                if lane.s >> k & 1 == 1 {
                    set(1 + l.system as u32 + k);
                }
            }
        }
    }
    // One stream per batch; a pass slot without a batch (the last pass) has no lanes, and its
    // stream (any stream) is never observable.
    let mut rngs: Vec<_> = (0..per)
        .map(|i| hmr_stream(&ctx.hmr_key, (first + i.min(chunks.len() - 1)) as u64))
        .collect();
    let t = prof::start();
    ex.run(&ctx.compiled.ops, &mut rngs);
    prof::stop(prof::EXEC, t);
    if ex.failed {
        return None;
    }
    // Any ancilla 1 at the end fails its lane (category 3 or earlier): re-run on the reference.
    let first_q = l.first_ancilla() as usize * WW;
    let hi = (ctx.compiled.num_qubits as usize * WW).min(ex.q.len());
    if ex.q[first_q.min(hi)..hi].iter().any(|&v| v != 0) {
        return None;
    }
    let gparams = ctx
        .tracker
        .and_then(|t| t.as_gaussian())
        .map(|g| (g.beta(), g.widths()));
    let mut key = Vec::new();
    let t_keys = prof::start();
    let mut total = 0;
    for (bi, chunk) in chunks.iter().enumerate() {
        total += chunk.len();
        let base = (first + bi) * LANES;
        for (i, lane) in chunk.iter().enumerate() {
            let at = bi * LANES + i;
            if !lane_passes::<WW>(
                ctx,
                nested,
                &ex,
                at,
                base + i,
                lane,
                gparams,
                memo,
                stats,
                &mut key,
            ) {
                return None;
            }
        }
    }
    prof::stop(prof::KEYS, t_keys);
    Some(Outcome {
        lanes: total,
        tally: ex.tally,
        ..Outcome::default()
    })
}

/// The reference's end-of-lane checks for the lane at pass position `at` (sample index
/// `index`): `true` when it passes them all.
#[allow(clippy::too_many_arguments)]
fn lane_passes<const WW: usize>(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    ex: &Exec<'_, WW>,
    at: usize,
    index: usize,
    lane: &Lane,
    gparams: Option<(u8, Option<&'static [u8]>)>,
    memo: &Memo,
    stats: &Stats,
    key: &mut Vec<u8>,
) -> bool {
    let l = &ctx.layout;
    let s_end = nested.map_or(lane.s, |n| n.after[index]);
    let c_now = get::<WW>(&ex.q, 0, at);
    let s_now = (0..l.uniform).fold(0u64, |a, k| {
        a | u64::from(get::<WW>(&ex.q, 1 + l.system as u32 + k, at)) << k
    });
    let s_want = if lane.c {
        (ctx.uniform_after)(s_end)
    } else {
        s_end
    };
    if c_now != lane.c || s_now != s_want {
        return false;
    }
    let frame = ex.lane_frame(at, true);
    let reference = lane.c.then(|| match nested {
        Some(n) => (n.reference)(lane.s, s_end),
        None => (ctx.reference)(lane.s),
    });
    let trace = &ex.traces[at];
    let monomial = reference.as_ref().and_then(SystemOp::as_monomial);
    if trace.is_empty() && (reference.is_none() || monomial.is_some()) {
        // No tracker and a Pauli reference: the reference's own Pauli checks.
        return match monomial {
            None => check_identity(&frame, l.system).is_none(),
            Some(m) => match monomial_to_frame(m, l.system) {
                Ok(want) => compare(&frame, &want, l.system).is_none(),
                Err(_) => false,
            },
        };
    }
    let Some(factory) = ctx.tracker else {
        return false;
    };
    stats.keyed_lanes.fetch_add(1, Ordering::Relaxed);
    key.clear();
    key.extend_from_slice(trace);
    key.push(0xff);
    put_frame(key, &frame);
    key.push(frame.phase);
    put_reference(key, reference.as_ref());
    let n = l.system;
    memo.get(key, reference.as_ref(), stats, || {
        let calls = decode_calls(trace, n);
        gauss::verdict(
            gparams,
            &stats.guard,
            &|| factory.lane(n),
            n,
            calls.iter().map(|(f, p, q, a)| gauss::Call {
                frame: f.as_ref(),
                p: *p,
                q: *q,
                angle: *a,
            }),
            &frame,
            reference.as_ref(),
        )
    })
}
