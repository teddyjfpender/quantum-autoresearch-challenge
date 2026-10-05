//! Per-lane checks (spec/DESIGN.md sections 6 and 9), run over the sampled lanes in parallel
//! batches of `LANES`.
//!
//! For every sampled lane `(c, s)` a passing run proves:
//! - no ancilla is 1 when an `R` frees it (checked during execution);
//! - the control qubit and the uniform register hold `(c, s)` again at the end, or
//!   `(1, uniform_after(s))` for a control-1 lane of a lane map that flips a swap bit
//!   (`thc-pair-alias-v1`, the THC encoding; no THC spec ships here); for a nested lane map `s` is first
//!   replaced by the second-pass value;
//! - every other non-system qubit is 0 at the end;
//! - for `c = 0` the system operator is the identity with phase `+1`;
//! - for `c = 1` it equals the lane map's `reference_op(s)` exactly, phase included.
//!
//! `run_nested` adds, for a nested lane map, the second-pass inner value of every lane: the
//! `Reflect` requires the inner register to hold the lane's own inner value and then replaces it
//! with the second-pass value, the uniform register must end holding the replaced value, and the
//! reference is `reference_nested(s, after)` (spec/DESIGN.md section 16).
use super::compile::{Compiled, Layout};
use super::jw::{monomial_to_frame, Frame};
use super::lanes::{Lanes, ReflectLanes, Tally, LANES, W};
use super::tracker::{LaneFrame, TrackerFactory};
use crate::fiat_shamir::{hmr_stream, Lane};
use crate::spec::SystemOp;
use rayon::prelude::*;

/// Failure categories, most specific first; the run reports the first category that occurred.
pub const CATEGORIES: [&str; 10] = [
    "dirty-ancilla-freed",
    "tracker",
    "registers-not-restored",
    "dirty-ancilla-at-end",
    "phase-garbage",
    "control-0-not-identity",
    "not-a-pauli",
    "wrong-phase",
    "wrong-term",
    "inner-register-at-reflect",
];

/// One failing lane.
#[derive(Clone, Debug)]
pub struct Failure {
    pub category: usize,
    /// Index of the lane in the sample.
    pub index: usize,
    pub lane: Lane,
    pub msg: String,
}

/// The outcome of running every sampled lane.
#[derive(Debug, Default)]
pub struct Outcome {
    pub lanes: usize,
    pub tally: Tally,
    /// Number of failing lanes per category.
    pub failed: [u64; 10],
    /// First failure (in sample order) per category.
    pub first: [Option<Failure>; 10],
}

impl Outcome {
    /// The most specific failure, if any lane failed.
    #[must_use]
    pub fn rejection(&self) -> Option<String> {
        let f = self.first.iter().flatten().next()?;
        let total: u64 = self.failed.iter().sum();
        Some(format!(
            "{}: lane #{} (c={}, s={}): {} [{total} of {} sampled lanes failed]",
            CATEGORIES[f.category],
            f.index,
            u8::from(f.lane.c),
            f.lane.s,
            f.msg,
            self.lanes
        ))
    }

    fn add(&mut self, f: Failure) {
        let c = f.category;
        self.failed[c] += 1;
        if self.first[c].as_ref().is_none_or(|g| f.index < g.index) {
            self.first[c] = Some(f);
        }
    }

    /// Order-independent combination of two batches' outcomes (sums; first failure by index).
    /// `pub(crate)` so the fast engine (`crate::fastsim`) merges exactly as `run` does.
    pub(crate) fn merge(mut self, o: Self) -> Self {
        self.lanes += o.lanes;
        let t = &mut self.tally;
        t.ccx += o.tally.ccx;
        t.ccz += o.tally.ccz;
        t.cliffords += o.tally.cliffords;
        t.hmr += o.tally.hmr;
        t.resets += o.tally.resets;
        t.givens += o.tally.givens;
        t.reflects += o.tally.reflects;
        t.spin_swaps += o.tally.spin_swaps;
        t.givens_charge += o.tally.givens_charge;
        for f in o.first.into_iter().flatten() {
            let c = f.category;
            self.failed[c] += o.failed[c] - 1;
            self.add(f);
        }
        self
    }
}

/// Everything a batch needs besides its lanes.
pub struct Context<'a> {
    pub compiled: &'a Compiled,
    pub layout: Layout,
    pub hmr_key: [u8; 32],
    pub reference: &'a (dyn Fn(u64) -> SystemOp + Sync),
    /// The uniform value a control-1 lane must end with (`LaneMap::uniform_after`).
    pub uniform_after: &'a (dyn Fn(u64) -> u64 + Sync),
    pub tracker: Option<&'a dyn TrackerFactory>,
}

/// What a nested lane map adds to each lane (spec/DESIGN.md section 16).
pub struct Nested<'a> {
    /// The inner uniform register: bits `lo .. lo + width`.
    pub lo: u32,
    pub width: u32,
    /// Per sampled lane (same order as the lanes): the uniform value after the `Reflect`.
    pub after: &'a [u64],
    /// `reference_nested(s, after)`.
    pub reference: &'a (dyn Fn(u64, u64) -> SystemOp + Sync),
}

/// Runs every lane in parallel batches and aggregates the outcome deterministically.
#[must_use]
pub fn run(ctx: &Context<'_>, lanes: &[Lane]) -> Outcome {
    run_with(ctx, None, lanes)
}

/// `run` for a nested lane map: `nested.after[i]` is lane `i`'s uniform value after the
/// `Reflect`.
#[must_use]
pub fn run_nested(ctx: &Context<'_>, nested: &Nested<'_>, lanes: &[Lane]) -> Outcome {
    run_with(ctx, Some(nested), lanes)
}

fn run_with(ctx: &Context<'_>, nested: Option<&Nested<'_>>, lanes: &[Lane]) -> Outcome {
    lanes
        .par_chunks(LANES)
        .enumerate()
        .map(|(b, chunk)| run_batch(ctx, nested, b, chunk, None))
        .reduce(Outcome::default, Outcome::merge)
}

/// Per-lane verdict of a lane that passed every check (`verdicts` of [`reference`]). A failing
/// lane's verdict is its category index into [`CATEGORIES`] (`0..10`).
pub const VERDICT_PASS: u8 = 100;
/// Per-lane verdict of a lane that was never checked: an execution error (a dirty `R`, a tracker
/// rejection, a `Reflect` mismatch) stopped its batch, and only the lane that raised it failed.
pub const VERDICT_UNCHECKED: u8 = 101;

/// A lane engine: what `score::evaluate_with` calls in place of [`run`] and [`run_nested`]. It
/// must return the reference's `Outcome` exactly and, when `verdicts` is given (same length as
/// `lanes`, sample order), write every lane's verdict as [`reference`] does. The equivalence
/// harness (`crate::equiv`, spec/FAST-EVALUATOR.md section 5) compares engines through this type.
pub type Engine = fn(&Context<'_>, Option<&Nested<'_>>, &[Lane], Option<&mut [u8]>) -> Outcome;

/// The reference engine, the default and the oracle: [`run`] (or [`run_nested`] when `nested`
/// is given), plus, when `verdicts` is given, each lane's verdict. Recording verdicts does not
/// change the `Outcome`: the same batches run the same code, and only the verdict bytes are
/// written besides.
#[must_use]
pub fn reference(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    lanes: &[Lane],
    verdicts: Option<&mut [u8]>,
) -> Outcome {
    let Some(v) = verdicts else {
        return run_with(ctx, nested, lanes);
    };
    assert_eq!(v.len(), lanes.len(), "one verdict per lane");
    lanes
        .par_chunks(LANES)
        .zip(v.par_chunks_mut(LANES))
        .enumerate()
        .map(|(b, (chunk, v))| run_batch(ctx, nested, b, chunk, Some(v)))
        .reduce(Outcome::default, Outcome::merge)
}

pub(crate) fn reflect_lanes(
    l: &super::Layout,
    n: &Nested<'_>,
    base: usize,
    chunk: &[Lane],
) -> ReflectLanes {
    let width = n.width as usize;
    let mut r = ReflectLanes {
        qubits: (0..n.width)
            .map(|j| 1 + l.system as u32 + n.lo + j)
            .collect(),
        want: vec![[0; W]; width],
        flip: vec![[0; W]; width],
    };
    for (i, lane) in chunk.iter().enumerate() {
        let (a, flip) = (lane.s >> n.lo, (lane.s ^ n.after[base + i]) >> n.lo);
        for j in 0..width {
            r.want[j][i / 64] |= (a >> j & 1) << (i % 64);
            r.flip[j][i / 64] |= (flip >> j & 1) << (i % 64);
        }
    }
    r
}

fn set(v: &mut [u64], q: u32, lane: usize) {
    v[q as usize * W + lane / 64] |= 1 << (lane % 64);
}

fn get(v: &[u64], q: u32, lane: usize) -> bool {
    v[q as usize * W + lane / 64] >> (lane % 64) & 1 == 1
}

/// One batch of the reference engine. `pub(crate)` so the fast engine (`crate::fastsim`) re-runs
/// any batch it does not pass with exactly this code.
pub(crate) fn run_batch(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    b: usize,
    chunk: &[Lane],
    mut verdicts: Option<&mut [u8]>,
) -> Outcome {
    let l = &ctx.layout;
    let mut sim = Lanes::new(ctx.compiled, l.system, ctx.tracker);
    sim.reflect = nested.map(|n| reflect_lanes(l, n, b * LANES, chunk));
    for (i, lane) in chunk.iter().enumerate() {
        sim.active[i / 64] |= 1 << (i % 64);
        if lane.c {
            set(&mut sim.q, 0, i);
        }
        for k in 0..l.uniform {
            if lane.s >> k & 1 == 1 {
                set(&mut sim.q, 1 + l.system as u32 + k, i);
            }
        }
    }
    let mut out = Outcome {
        lanes: chunk.len(),
        ..Outcome::default()
    };
    let base = b * LANES;
    let mut rng = hmr_stream(&ctx.hmr_key, b as u64);
    if let Err(e) = sim.run(&ctx.compiled.ops, &mut rng) {
        let category = match e.kind {
            "dirty-ancilla" => 0,
            "reflect" => 9,
            _ => 1,
        };
        if let Some(v) = verdicts.as_deref_mut() {
            v.fill(VERDICT_UNCHECKED);
            if let Some(x) = v.get_mut(e.lane) {
                *x = category as u8;
            }
        }
        out.add(Failure {
            category,
            index: base + e.lane,
            lane: chunk.get(e.lane).copied().unwrap_or(chunk[0]),
            msg: e.msg,
        });
        out.tally = sim.tally;
        return out;
    }
    out.tally = sim.tally;
    let dirty = dirty_lanes(&sim, l.first_ancilla(), ctx.compiled.num_qubits);
    for (i, lane) in chunk.iter().enumerate() {
        let dirty = dirty[i / 64] >> (i % 64) & 1 == 1;
        let verdict = check_lane(ctx, nested, &mut sim, base + i, i, *lane, dirty);
        if let Some(v) = verdicts.as_deref_mut() {
            v[i] = verdict.as_ref().map_or(VERDICT_PASS, |(c, _)| *c as u8);
        }
        if let Some((category, msg)) = verdict {
            out.add(Failure {
                category,
                index: base + i,
                lane: *lane,
                msg,
            });
        }
    }
    out
}

/// The lanes on which some ancilla (qubit `first..num_qubits`) is 1, as one word pass over the
/// batch. Scanning every ancilla per lane cost `num_qubits` per lane, so one op naming qubit
/// `2^20 - 1` made a 3-op circuit evaluate 20x slower than a 5000-op one.
fn dirty_lanes(sim: &Lanes<'_>, first: u32, num_qubits: u32) -> [u64; W] {
    let mut dirty = [0u64; W];
    let lo = (first as usize * W).min(sim.q.len());
    let hi = (num_qubits as usize * W).min(sim.q.len());
    for q in sim.q[lo..hi].chunks_exact(W) {
        for (d, &v) in dirty.iter_mut().zip(q) {
            *d |= v;
        }
    }
    dirty
}

fn check_lane(
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    sim: &mut Lanes<'_>,
    global: usize,
    i: usize,
    lane: Lane,
    dirty: bool,
) -> Option<(usize, String)> {
    let l = &ctx.layout;
    let s_end = nested.map_or(lane.s, |n| n.after[global]);
    let c_now = get(&sim.q, 0, i);
    let s_now = (0..l.uniform).fold(0u64, |a, k| {
        a | u64::from(get(&sim.q, 1 + l.system as u32 + k, i)) << k
    });
    // Nested lanes end at the second-pass value; a swap-bit lane map then maps that value.
    let s_want = if lane.c {
        (ctx.uniform_after)(s_end)
    } else {
        s_end
    };
    if c_now != lane.c || s_now != s_want {
        let msg = format!(
            "control/uniform end as (c={}, s={s_now}); expected (c={}, s={s_want})",
            u8::from(c_now),
            u8::from(lane.c)
        );
        return Some((2, msg));
    }
    let first_dirty = || (l.first_ancilla()..ctx.compiled.num_qubits).find(|&q| get(&sim.q, q, i));
    if let Some(q) = dirty.then(first_dirty).flatten() {
        return Some((
            3,
            format!("qubit q{q} is 1 at the end; every ancilla must be |0>"),
        ));
    }
    let frame = sim.lane_frame(i, true);
    let reference = lane.c.then(|| match nested {
        Some(n) => (n.reference)(lane.s, s_end),
        None => (ctx.reference)(lane.s),
    });
    if let Some(tr) = sim.trackers[i].as_mut() {
        return tr.finish(&frame, reference.as_ref()).err().map(|m| (1, m));
    }
    match reference {
        None => check_identity(&frame, l.system),
        Some(r) => match r.as_monomial() {
            Some(m) => match monomial_to_frame(m, l.system) {
                Ok(want) => compare(&frame, &want, l.system),
                Err(e) => Some((8, format!("reference op invalid: {e}"))),
            },
            None => match ctx.tracker {
                Some(f) => f
                    .lane(l.system)
                    .finish(&frame, Some(&r))
                    .err()
                    .map(|m| (1, m)),
                None => Some((
                    1,
                    "reference op needs the df tracker (not in this build)".into(),
                )),
            },
        },
    }
}

fn to_frame(f: &LaneFrame) -> Frame {
    Frame {
        x: f.x.clone(),
        z: f.z.clone(),
        phase: f.phase,
    }
}

fn not_pauli(f: &LaneFrame) -> Option<(usize, String)> {
    let q = f.s_pow.iter().position(|&s| s != 0)?;
    Some((
        6,
        format!(
            "system qubit {q} is left with S^{}: not a Pauli",
            f.s_pow[q]
        ),
    ))
}

pub(crate) fn check_identity(f: &LaneFrame, n: usize) -> Option<(usize, String)> {
    if let Some(e) = not_pauli(f) {
        return Some(e);
    }
    let got = to_frame(f);
    if got.is_identity() && f.phase != 0 {
        return Some((
            4,
            format!("control-0 lane applies w^{} * I, not +I", f.phase),
        ));
    }
    (!got.is_identity()).then(|| (5, format!("control-0 lane applies {}", got.describe(n))))
}

pub(crate) fn compare(f: &LaneFrame, want: &Frame, n: usize) -> Option<(usize, String)> {
    if let Some(e) = not_pauli(f) {
        return Some(e);
    }
    let got = to_frame(f);
    if got == *want {
        return None;
    }
    let detail = format!(
        "applies {} but the lane map requires {}",
        got.describe(n),
        want.describe(n)
    );
    if got.x != want.x || got.z != want.z {
        return Some((8, detail));
    }
    let off = (8 + got.phase - want.phase) % 8;
    let what = if off == 4 {
        "sign flipped".to_string()
    } else {
        format!("phase off by w^{off}")
    };
    Some((7, format!("{what}: {detail}")))
}
