//! The fermionic-Gaussian tracker of `crate::sim::gaussian` with faster final-check kernels.
//!
//! `FastLane` is a field-for-field copy of `GaussianLane`: `givens_modes` and the frame
//! bookkeeping are the reference code verbatim, so the tracked state (`u`, the vectors, the
//! phase) is bit-identical. Only the two kernels of `measure` differ, and only in *schedule*:
//!
//! - `ad_residual`: every basis column's reflection chain runs exactly the reference's float
//!   operations in the reference's order (sequential `Sum` fold from the toolchain's own start
//!   value, then `d * w - y`); columns are independent, so running `CB` of them side by side
//!   (SIMD lanes) changes no bit. The maximum of non-negative, non-NaN values does not depend on
//!   order, and any NaN makes both return `INFINITY`.
//! - the Pfaffian: the contraction matrix's upper triangle is computed exactly as the reference
//!   computes it. The elimination stores only the upper triangle. The reference's lower entry
//!   `(j, i)` receives `tau_i row_j - tau_j row_i`, the exact negation of the upper update
//!   (IEEE subtraction is antisymmetric) except that `x - x` is `+0` both ways, so every lower
//!   entry equals the negated upper entry *up to the sign of a zero*. With every value finite,
//!   values equal up to zero signs give products, sums, quotients (by `|head|^2 > 0`), `hypot`
//!   norms, pivot choices and the head test that are equal up to zero signs, so the returned
//!   overlap equals the reference's up to zero signs and `nearest_w` (a norm of a difference)
//!   is identical. If any value read is not finite, the kernel gives up and the caller uses the
//!   reference `GaussianLane` itself.
//!
//! The verdict (`check`) is therefore the reference `finish`'s verdict on every input. Failure
//! *messages* are not produced here: the fast engine re-runs any batch with a failing lane on the
//! reference engine, which produces them.
use super::kernels;
use crate::sim::gaussian::pfaffian::C64;
use crate::sim::gaussian::{
    cos_sin, frame_to_majoranas, givens_scale, reference_vectors, GaussianLane, Tv, AD_TOL,
    MAX_FACTORS, PHASE_TOL,
};
use crate::sim::tracker::{LaneFrame, LaneTracker};
use crate::spec::SystemOp;
use std::sync::OnceLock;

/// A copy of `GaussianLane` (same fields, same `givens_modes`) with the fast kernels.
pub struct FastLane {
    n: usize,
    beta: u8,
    widths: Option<&'static [u8]>,
    u: Option<Vec<f64>>,
    /// The tracked vectors, built from the slots by `build_vecs` after the last frame.
    vecs: Vec<Tv>,
    /// Per tracked vector (in hand-off order): its slot and its block.
    slot: Vec<usize>,
    odd: Vec<bool>,
    /// The distinct tracked vectors ("slots"), component-major: component `r` of slot `s` at
    /// `cols[r * cap + s]`. A frame's pair `gamma_2q gamma_2q+1` starts as two copies of `e_q`
    /// that every later rotation maps by the same float operations, so it is held once.
    cols: Vec<f64>,
    cap: usize,
    slots: usize,
    /// Per slot, the running bound (`eps` of each of its vectors).
    seps: Vec<f64>,
    chunks: Vec<usize>,
    phase: u8,
    /// Not in the reference: per vector, a running bound on its distance from the ideal vector
    /// (`fold` module docs); `eps_u` bounds the mode rotation's columns. Floats of the tracked
    /// state are untouched by these.
    eps: Vec<f64>,
    eps_u: f64,
    /// When set, the Majoranas of every frame handed over, in order (the fold replays them
    /// instead of converting each frame again).
    ms_log: Option<Vec<Vec<u16>>>,
    /// When set, the mode rotation is not formed (only the fold's guard reads it, as column
    /// supports): `rows[r]` is a superset, as a bit set over columns, of the columns whose
    /// entry in row `r` may be non-zero. `u_started` records that a rotation ran.
    rows: Option<Vec<u64>>,
    u_started: bool,
}

impl FastLane {
    #[must_use]
    pub fn new(n: usize, beta: u8, widths: Option<&'static [u8]>) -> Self {
        Self {
            n,
            beta,
            widths,
            u: None,
            vecs: Vec::new(),
            slot: Vec::new(),
            odd: Vec::new(),
            cols: Vec::new(),
            cap: 0,
            slots: 0,
            seps: Vec::new(),
            chunks: Vec::new(),
            phase: 0,
            eps: Vec::new(),
            eps_u: 0.0,
            ms_log: None,
            rows: None,
            u_started: false,
        }
    }

    /// `GaussianLane::push_frame` (the same factors, as slots).
    fn push_frame(&mut self, f: &LaneFrame) -> Result<(), String> {
        let (ph, ms) = frame_to_majoranas(f, self.n)?;
        if let Some(log) = &mut self.ms_log {
            log.push(ms.clone());
        }
        self.phase = (self.phase + ph) % 8;
        if ms.is_empty() {
            return Ok(());
        }
        if self.slot.len() + ms.len() > MAX_FACTORS {
            return Err(format!(
                "tracker: more than {MAX_FACTORS} Majorana factors on one lane (not supported)"
            ));
        }
        self.chunks.push(self.slot.len());
        let mut prev: Option<usize> = None;
        for &m in &ms {
            let m = usize::from(m);
            // `Tv::basis(m)`: `e_{m / 2}` in block `m % 2`; the odd one of a pair shares the
            // even one's slot.
            let s = match prev {
                Some(p) if m % 2 == 1 && p + 1 == m => self.slot[self.slot.len() - 1],
                _ => self.new_slot(m / 2),
            };
            self.slot.push(s);
            self.odd.push(m % 2 == 1);
            prev = Some(m);
        }
        Ok(())
    }

    /// A new slot holding `e_q`.
    fn new_slot(&mut self, q: usize) -> usize {
        let n = self.n;
        if self.slots == self.cap {
            let cap = (2 * self.cap).max(32);
            let mut cols = vec![0.0; n * cap];
            for r in 0..n {
                cols[r * cap..r * cap + self.slots]
                    .copy_from_slice(&self.cols[r * self.cap..r * self.cap + self.slots]);
            }
            self.cols = cols;
            self.cap = cap;
        }
        let s = self.slots;
        for r in 0..n {
            self.cols[r * self.cap + s] = 0.0;
        }
        self.cols[q * self.cap + s] = 1.0;
        self.seps.push(0.0);
        self.slots += 1;
        s
    }

    /// The tracked vectors (`GaussianLane`'s `vecs`) and their bounds, from the slots.
    fn build_vecs(&mut self) {
        let (n, cap) = (self.n, self.cap);
        let cols: Vec<Vec<f64>> = (0..self.slots)
            .map(|s| (0..n).map(|r| self.cols[r * cap + s]).collect())
            .collect();
        self.vecs = self
            .slot
            .iter()
            .zip(&self.odd)
            .map(|(&s, &odd)| Tv {
                odd,
                v: cols[s].clone(),
            })
            .collect();
        self.eps = self.slot.iter().map(|&s| self.seps[s]).collect();
    }

    /// `GaussianLane::x_factors`, verbatim.
    fn x_factors(&self) -> Vec<&Tv> {
        let mut out = Vec::with_capacity(self.vecs.len());
        let mut end = self.vecs.len();
        for &start in self.chunks.iter().rev() {
            out.extend(&self.vecs[start..end]);
            end = start;
        }
        out
    }

    /// `GaussianLane::givens_modes`, verbatim.
    ///
    /// # Errors
    /// As the reference.
    pub fn givens_modes(
        &mut self,
        before: Option<&LaneFrame>,
        p: usize,
        q: usize,
        angle: u64,
    ) -> Result<(), String> {
        if p >= q || q >= self.n {
            return Err(format!(
                "Givens on modes ({p}, {q}) needs p < q < {}",
                self.n
            ));
        }
        // The identity frame adds phase 0 and no factor (`frame_to_majoranas`), so skipping it
        // leaves the state as the reference's.
        if let Some(before) = before {
            self.push_frame(before)?;
        }
        let w = givens_scale(self.beta, self.widths, p, q);
        let angle = if w < self.beta {
            (angle & ((1u64 << w) - 1)) << (self.beta - w)
        } else {
            angle
        };
        let (c, s) = cos_sin(angle, self.beta);
        let n = self.n;
        self.u_started = true;
        if let Some(rows) = &mut self.rows {
            // Rows p and q both take the union of their column sets (an exact zero stays one
            // only where both inputs are zero).
            let words = n.div_ceil(64);
            for w in 0..words {
                let v = rows[p * words + w] | rows[q * words + w];
                rows[p * words + w] = v;
                rows[q * words + w] = v;
            }
        } else {
            let u = self.u.get_or_insert_with(|| {
                let mut m = vec![0.0; n * n];
                (0..n).for_each(|i| m[i * n + i] = 1.0);
                m
            });
            for j in 0..n {
                let (a, b) = (u[p * n + j], u[q * n + j]);
                u[p * n + j] = c * a - s * b;
                u[q * n + j] = s * a + c * b;
            }
        }
        let (cap, k) = (self.cap, self.slots);
        let (lo, hi) = self.cols.split_at_mut(q * cap);
        let (rp, rq) = (&mut lo[p * cap..p * cap + k], &mut hi[..k]);
        if bounds() {
            let kap = super::fold::kappa(c, s).unwrap_or(f64::INFINITY);
            self.eps_u += std::f64::consts::SQRT_2 * kap * (1.0 + self.eps_u);
            for ((e, a), b) in self.seps.iter_mut().zip(rp.iter()).zip(rq.iter()) {
                *e += kap * (a.abs() + b.abs());
            }
        }
        // `rotate` on every slot: the same float operations.
        for (a, b) in rp.iter_mut().zip(rq.iter_mut()) {
            let (x, y) = (*a, *b);
            *a = c * x - s * y;
            *b = s * x + c * y;
        }
        Ok(())
    }

    /// The factors of `X` in product order with their bounds; chunk `g` is group
    /// `first_group + g`.
    fn x_factor_bounds(&self, first_group: usize) -> Vec<super::fold::Factor<'_>> {
        let mut out = Vec::with_capacity(self.vecs.len());
        let mut end = self.vecs.len();
        for (g, &start) in self.chunks.iter().enumerate().rev() {
            for i in start..end {
                out.push(super::fold::Factor {
                    y: &self.vecs[i],
                    eps: self.eps[i],
                    group: first_group + g,
                });
            }
            end = start;
        }
        out
    }

    /// `GaussianLane::measure` with the fast kernels: `Ok(None)` when the Pfaffian kernel met a
    /// non-finite value (the caller then uses the reference).
    ///
    /// # Errors
    /// As the reference (an unsupported final frame or a malformed reference).
    pub fn measure(
        &mut self,
        after: &LaneFrame,
        reference: Option<&SystemOp>,
    ) -> Result<Option<(f64, C64)>, String> {
        self.push_frame(after)?;
        self.build_vecs();
        let (ref_phase, refs) = reference_vectors(reference, self.n)?;
        let mut ys: Vec<&Tv> = refs.iter().rev().collect();
        ys.extend(self.x_factors());
        let k = (8 + self.phase - ref_phase % 8) % 8;
        let t = super::prof::start();
        let off = kernels::ad_residual(self.u.as_deref(), &ys, self.n);
        super::prof::stop(super::prof::AD, t);
        let ve = kernels::vacuum_expectation(&ys);
        Ok(ve.map(|c| (off, c.scale_w(k))))
    }
}

/// The identity frame on `n` qubits, as `Lanes::lane_frame` extracts it.
#[must_use]
pub fn identity_frame(n: usize) -> LaneFrame {
    LaneFrame {
        x: vec![0; n.div_ceil(64)],
        z: vec![0; n.div_ceil(64)],
        s_pow: vec![0; n],
        phase: 0,
    }
}

/// `GaussianLane::finish`'s verdict from `measure`'s values (`true`: accepted).
#[must_use]
pub fn accepts(off: f64, c: C64) -> bool {
    if off > AD_TOL {
        return false;
    }
    matches!(c.nearest_w(), (0, d) if d <= PHASE_TOL)
}

/// One trace replay's input: a `Givens` call (frame, `p`, `q`, angle).
pub struct Call<'a> {
    /// `None`: the identity frame (which `push_frame` turns into no factor and phase 0).
    pub frame: Option<&'a LaneFrame>,
    pub p: usize,
    pub q: usize,
    pub angle: u64,
}

/// The reference `finish` verdict for a lane whose tracker received `calls` and whose final
/// frame is `after`: computed with `FastLane` when `params` names the Gaussian factory, with
/// the reference `GaussianLane` when a kernel declines, and with `generic` (the factory's own
/// `lane`) for any other tracker.
pub fn verdict<'a>(
    params: Option<(u8, Option<&'static [u8]>)>,
    guard: &super::fold::GuardCounts,
    generic: &dyn Fn() -> Box<dyn LaneTracker>,
    n: usize,
    calls: impl Iterator<Item = Call<'a>> + Clone,
    after: &LaneFrame,
    reference: Option<&SystemOp>,
) -> bool {
    let identity = identity_frame(n);
    let reference_path = |mut tr: Box<dyn LaneTracker>| -> bool {
        for c in calls.clone() {
            if tr
                .givens_modes(c.frame.unwrap_or(&identity), c.p, c.q, c.angle)
                .is_err()
            {
                return false;
            }
        }
        tr.finish(after, reference).is_ok()
    };
    let Some((beta, widths)) = params else {
        return reference_path(generic());
    };
    if bounds() {
        return match folded(beta, widths, n, calls.clone(), after, reference, guard) {
            Some(v) => v,
            None => reference_path(generic()),
        };
    }
    let mut fl = FastLane::new(n, beta, widths);
    let t = super::prof::start();
    for c in calls.clone() {
        if fl.givens_modes(c.frame, c.p, c.q, c.angle).is_err() {
            return false;
        }
    }
    super::prof::stop(super::prof::TRACK, t);
    let fast = fl.measure(after, reference);
    if selfcheck() {
        cross_check(&fast, beta, widths, n, calls.clone(), after, reference);
    }
    match fast {
        Err(_) => false,
        Ok(Some((off, c))) => accepts(off, c),
        // The factory's own lane is the reference `GaussianLane` (with its widths).
        Ok(None) => reference_path(generic()),
    }
}

/// Below this many tracked vectors the fold is not tried (a performance choice only;
/// `FEMOCO_FASTSIM_FOLD_MIN` overrides it, e.g. 0 to fold every lane in tests).
fn fold_min() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("FEMOCO_FASTSIM_FOLD_MIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16)
    })
}

/// Whether the guarded fold is on (`FEMOCO_FASTSIM_FOLD=0` turns it off; tooling for A/B
/// measurements, verdicts are identical either way).
#[must_use]
pub fn bounds() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("FEMOCO_FASTSIM_FOLD").map_or(true, |v| v != "0"))
}

/// The reference verdict with the `Ad` residual taken from the guarded fold when it certifies
/// (`fold` module docs), and from the exact kernel otherwise. The vacuum overlap is always the
/// exact kernel's (no rigorous guard exists for the reference's pivoted elimination; docs
/// section 6.1). `None`: a kernel declined (the caller uses the reference tracker).
fn folded<'a>(
    beta: u8,
    widths: Option<&'static [u8]>,
    n: usize,
    calls: impl Iterator<Item = Call<'a>> + Clone,
    after: &LaneFrame,
    reference: Option<&SystemOp>,
    guard: &super::fold::GuardCounts,
) -> Option<bool> {
    let p = fold_probe(beta, widths, n, calls, after, reference, selfcheck(), guard)?;
    if selfcheck() {
        if let (Some(g), Some(off)) = (p.g, p.off_ref) {
            assert!(
                (off - p.off_fold).abs() <= g,
                "fastsim selfcheck: |off_ref {off:e} - off_fold {:e}| > g {g:e}",
                p.off_fold
            );
            super::fold::note_ratio((off - p.off_fold).abs() / g);
        }
    }
    Some(p.verdict)
}

/// One guarded-fold evaluation (`folded`), with its parts for tests and the self-check.
pub struct Probe {
    pub verdict: bool,
    /// The guard certified the fold's residual (the exact residual was not needed).
    pub certified: bool,
    pub off_fold: f64,
    /// `g` (`None`: no bound, a fallback).
    pub g: Option<f64>,
    /// The exact residual, when it was computed (a fallback, or `always_exact`).
    pub off_ref: Option<f64>,
}

/// The reference's mode rotation `U` for `calls` (the frames do not enter it): `fl`'s own
/// when it formed one, else the same rotations replayed with the reference's float operations.
fn replay_u<'a, 'b>(
    fl: &'b FastLane,
    calls: impl Iterator<Item = Call<'a>>,
) -> Option<std::borrow::Cow<'b, [f64]>> {
    if fl.u.is_some() || !fl.u_started {
        return fl.u.as_deref().map(std::borrow::Cow::Borrowed);
    }
    let mut g = FastLane::new(fl.n, fl.beta, fl.widths);
    for c in calls {
        // Frames only add vectors; skipping them leaves `U` unchanged.
        if g.givens_modes(None, c.p, c.q, c.angle).is_err() {
            return None;
        }
    }
    g.u.map(std::borrow::Cow::Owned)
}

#[allow(clippy::too_many_arguments)]
/// `folded`'s computation. `always_exact` also computes the exact residual when the guard
/// certifies (tests and the self-check), without changing the verdict's source.
pub fn fold_probe<'a>(
    beta: u8,
    widths: Option<&'static [u8]>,
    n: usize,
    calls: impl Iterator<Item = Call<'a>> + Clone,
    after: &LaneFrame,
    reference: Option<&SystemOp>,
    always_exact: bool,
    guard: &super::fold::GuardCounts,
) -> Option<Probe> {
    use super::fold::{self, FoldLane};
    use super::prof;
    let reject = Probe {
        verdict: false,
        certified: false,
        off_fold: f64::NAN,
        g: None,
        off_ref: None,
    };
    let mut fl = FastLane::new(n, beta, widths);
    fl.ms_log = Some(Vec::new());
    // An upper bound on the vectors the frames will add (two Majoranas per qubit with an X, Z
    // or S^2 factor): below the fold's threshold the exact residual will be needed anyway, so
    // `U` is formed directly. A performance choice only.
    let majoranas: usize = calls
        .clone()
        .filter_map(|c| c.frame)
        .chain([after])
        .map(|f| {
            let xz: u32 =
                f.x.iter()
                    .zip(&f.z)
                    .map(|(x, z)| (x | z).count_ones())
                    .sum();
            2 * (xz as usize + f.s_pow.iter().filter(|&&p| p % 4 == 2).count())
        })
        .sum();
    if !always_exact && majoranas >= fold_min() {
        // The mode rotation is formed only if the exact residual is needed (`replay_u`).
        let words = n.div_ceil(64);
        let mut rows = vec![0u64; n * words];
        for r in 0..n {
            rows[r * words + r / 64] |= 1 << (r % 64);
        }
        fl.rows = Some(rows);
    }
    let t = prof::start();
    for c in calls.clone() {
        if fl.givens_modes(c.frame, c.p, c.q, c.angle).is_err() {
            return Some(reject);
        }
    }
    prof::stop(prof::TRACK, t);
    if fl.push_frame(after).is_err() {
        return Some(reject);
    }
    fl.build_vecs();
    let log = fl.ms_log.take().unwrap_or_default();
    let Ok((ref_phase, refs, rb)) = fold::reference_factors(reference, n) else {
        return Some(reject);
    };
    let mut ys: Vec<&Tv> = refs.iter().rev().collect();
    ys.extend(fl.x_factors());
    let k = (8 + fl.phase - ref_phase % 8) % 8;
    let ve = kernels::vacuum_expectation(&ys)?;
    if selfcheck() {
        let want = crate::sim::gaussian::pfaffian::vacuum_expectation(&ys);
        assert!(
            same_up_to_zero_sign(ve.re, want.re) && same_up_to_zero_sign(ve.im, want.im),
            "fastsim selfcheck: overlap {ve:?} vs reference {want:?}"
        );
    }
    let c = ve.scale_w(k);
    let phase_ok = matches!(c.nearest_w(), (0, d) if d <= PHASE_TOL);
    if !phase_ok && !always_exact {
        // The reference rejects whatever its residual is.
        return Some(reject);
    }
    if fl.vecs.len() < fold_min() && !always_exact {
        // Few factors: the exact residual costs about what the fold would.
        guard
            .small
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let u = replay_u(&fl, calls.clone());
        let t = prof::start();
        let off = kernels::ad_residual(u.as_deref(), &ys, n);
        prof::stop(prof::AD, t);
        return Some(Probe {
            verdict: phase_ok && off <= AD_TOL,
            certified: false,
            off_fold: f64::NAN,
            g: None,
            off_ref: Some(off),
        });
    }
    let t = prof::start();
    let mut fd = FoldLane::new(n, beta, widths);
    // The frames' Majoranas as `fl` converted them, in hand-off order (one per non-identity
    // frame, then the final frame).
    let calls_again = calls.clone();
    let mut logged = log.iter();
    for c in calls {
        let ms = match c.frame {
            Some(_) => Some(logged.next()?.as_slice()),
            None => None,
        };
        fd.givens_modes_ms(ms, c.p, c.q, c.angle);
    }
    fd.push_ms(logged.next()?);
    let first = rb.iter().map(|&(_, g)| g + 1).max().unwrap_or(0);
    let mut yr = Vec::with_capacity(rb.len() + fl.vecs.len());
    let mut yf = Vec::with_capacity(rb.len() + fd.vecs.len());
    for (y, &(eps, group)) in refs.iter().zip(&rb).rev() {
        yr.push(fold::Factor { y, eps, group });
        yf.push(fold::Factor { y, eps, group });
    }
    yr.extend(fl.x_factor_bounds(first));
    yf.extend(fd.x_factors(first));
    let ys_f: Vec<&Tv> = yf.iter().map(|f| f.y).collect();
    let off_fold = kernels::ad_residual(fd.u.as_deref(), &ys_f, n);
    let ref_cols = match (&fl.rows, fl.u.as_deref()) {
        (_, Some(u)) => fold::Cols::Matrix(u),
        (Some(rows), None) if fl.u_started => fold::Cols::Rows(rows),
        _ => fold::Cols::Identity,
    };
    let fold_cols =
        fd.u.as_deref()
            .map_or(fold::Cols::Identity, fold::Cols::Matrix);
    let g = match (
        fold::gap(ref_cols, fl.eps_u, &yr, n),
        fold::gap(fold_cols, fd.eps_u, &yf, n),
    ) {
        (Some(a), Some(b)) => Some((a + b) * (1.0 + 4.0 * fold::U)),
        _ => None,
    };
    let certified = fold::certifies(off_fold, g);
    prof::stop(prof::FOLD, t);
    fold::note(guard, certified, g);
    let off_ref = (!certified || always_exact).then(|| {
        let u = replay_u(&fl, calls_again.clone());
        let t = prof::start();
        let off = kernels::ad_residual(u.as_deref(), &ys, n);
        prof::stop(prof::AD, t);
        off
    });
    let ad_ok = certified || off_ref.is_some_and(|off| off <= AD_TOL);
    Some(Probe {
        verdict: phase_ok && ad_ok,
        certified,
        off_fold,
        g,
        off_ref,
    })
}

/// `FEMOCO_FASTSIM_SELFCHECK=1`: every fast `measure` is compared with the reference
/// `GaussianLane::measure` on the same calls (tooling for the audit; panics on a mismatch).
fn selfcheck() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FEMOCO_FASTSIM_SELFCHECK").is_some())
}

/// Equal up to the sign of a zero.
#[must_use]
pub fn same_up_to_zero_sign(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a == 0.0 && b == 0.0)
}

fn cross_check<'a>(
    fast: &Result<Option<(f64, C64)>, String>,
    beta: u8,
    widths: Option<&'static [u8]>,
    n: usize,
    calls: impl Iterator<Item = Call<'a>>,
    after: &LaneFrame,
    reference: Option<&SystemOp>,
) {
    let mut g = GaussianLane::new(n, beta);
    g.set_widths(widths);
    let mut err = None;
    let identity = identity_frame(n);
    for c in calls {
        if let Err(e) = g.givens_modes(c.frame.unwrap_or(&identity), c.p, c.q, c.angle) {
            err = Some(e);
            break;
        }
    }
    let want = match err {
        Some(e) => Err(e),
        None => g.measure(after, reference),
    };
    match (fast, &want) {
        (Ok(Some((off, c))), Ok((woff, wc))) => assert!(
            off.to_bits() == woff.to_bits()
                && same_up_to_zero_sign(c.re, wc.re)
                && same_up_to_zero_sign(c.im, wc.im),
            "fastsim selfcheck: fast ({off:e}, {c:?}) vs reference ({woff:e}, {wc:?})"
        ),
        (Ok(None), _) | (Err(_), Err(_)) => {}
        _ => panic!("fastsim selfcheck: fast {fast:?} vs reference {want:?}"),
    }
}
