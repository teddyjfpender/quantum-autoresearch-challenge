//! Fermionic-Gaussian tracking of `Givens` (spec/DESIGN.md section 15).
//!
//! A lane's system operator is `O = w^phase F_{m+1} G_m F_m ... G_1 F_1`, where each `F_j` is the
//! Pauli frame the simulator hands over at a `Givens` and each `G_j` is a Givens rotation
//! `exp(theta (a+_q a_p - a+_p a_q))`. Moving every frame to the left gives `O = w^phase X P`:
//! `P = G_m ... G_1` is number conserving, so `P |vac> = |vac>` exactly and its adjoint action
//! on Majorana vectors is `U (+) U` for the real mode rotation `U` (tracked as an `n x n`
//! matrix); `X` is the product of the frames' Majoranas, each rotated by every later Givens
//! (tracked as vectors, two components per rotation).
//!
//! `finish` compares `O` with the reference `O_ref` (a Majorana monomial or a `Rotated` op) by
//! forming `Omega = O_ref^-1 O = w^k Y P` with `Y` a product of Majorana vectors, and proves:
//! 1. `Ad(Omega) = Ad(Y) (U (+) U)` equals the identity on every basis vector to within
//!    `AD_TOL` (so `Omega` is a scalar up to that tolerance; conjugation alone cannot see the
//!    scalar), and
//! 2. that scalar is `+1`: `<vac| Omega |vac> = w^k <vac| Y |vac>` (because `P |vac> = |vac>`),
//!    and `<vac| Y |vac>` is a Pfaffian of vacuum contractions (Wick), computed to `PHASE_TOL`.
//!    The candidates `w^j` are at least `|1 - w| = 0.765` apart, so the sign is exact.
mod frames;
pub mod pfaffian;
mod reference;

use super::tracker::{LaneFrame, LaneTracker, TrackerFactory};
use crate::spec::SystemOp;
pub use frames::frame_to_majoranas;
pub use reference::reference_vectors;

/// Largest entry of `Ad(Omega) - I` accepted. Measured float64 residual on correct lanes of the
/// pinned specs: at most 7.3e-15; one angle unit off: at least 1.5e-6 (spec/DESIGN.md section
/// 15, which also bounds what a residual below this tolerance can hide).
pub const AD_TOL: f64 = 1e-11;
/// Largest `|<vac|Omega|vac> - 1|` accepted.
pub const PHASE_TOL: f64 = 1e-6;
/// Majorana factors a lane may accumulate (bounds the Pfaffian's cost; more is rejected).
pub const MAX_FACTORS: usize = 512;

/// A Majorana vector `gamma(v) = sum_p v_p gamma_{2p + odd}`: every tracked vector lies in the
/// even or the odd block, because Givens rotations act on both blocks alike.
#[derive(Clone, Debug)]
pub struct Tv {
    pub odd: bool,
    pub v: Vec<f64>,
}

impl Tv {
    #[must_use]
    pub fn basis(majorana: usize, n: usize) -> Self {
        let mut v = vec![0.0; n];
        v[majorana / 2] = 1.0;
        Self {
            odd: majorana % 2 == 1,
            v,
        }
    }
}

/// `(cos theta, sin theta)` for `theta = 2 pi a / 2^beta`.
#[must_use]
pub fn cos_sin(a: u64, beta: u8) -> (f64, f64) {
    let a = a & ((1u64 << beta) - 1);
    let th = a as f64 * (std::f64::consts::TAU / (1u64 << beta) as f64);
    (th.cos(), th.sin())
}

/// The mode rotation of `G_{p,q}(theta)` on a vector: `e_p -> c e_p + s e_q`,
/// `e_q -> c e_q - s e_p`.
#[inline]
pub fn rotate(v: &mut [f64], p: usize, q: usize, c: f64, s: f64) {
    let (a, b) = (v[p], v[q]);
    v[p] = c * a - s * b;
    v[q] = s * a + c * b;
}

/// The tracker for one rotation precision `beta` (angle register value `a` means
/// `theta = 2 pi a / 2^beta`), or for a tapered sos-sa spec (spec/SPEC-SA.md section 13): with
/// `widths`, a `Givens` on modes `(2 j + s, 2 j + 2 + s)` (chain position `j`, spin `s`) reads its
/// register as the top `widths[j]` bits of the angle, `theta = 2 pi a / 2^widths[j]`, and is
/// charged `2 (widths[j] - 2)`; every other `Givens` keeps `beta`.
pub struct GaussianFactory {
    beta: u8,
    widths: Option<&'static [u8]>,
}

const fn factories() -> [GaussianFactory; 33] {
    let mut out = [const {
        GaussianFactory {
            beta: 0,
            widths: None,
        }
    }; 33];
    let mut b = 0;
    while b < 33 {
        out[b] = GaussianFactory {
            beta: b as u8,
            widths: None,
        };
        b += 1;
    }
    out
}

/// The tracker for a tapered spec (spec/SPEC-SA.md section 13): `beta = max widths`, `3 <= w_j`.
/// One factory per distinct schedule, kept for the life of the process.
#[must_use]
pub fn tapered_factory(beta: u8, widths: &[u8]) -> Option<&'static GaussianFactory> {
    static CACHE: std::sync::Mutex<Vec<&'static GaussianFactory>> =
        std::sync::Mutex::new(Vec::new());
    if !(3..=32).contains(&beta) || widths.iter().any(|&w| !(3..=beta).contains(&w)) {
        return None;
    }
    let mut cache = CACHE.lock().ok()?;
    if let Some(f) = cache
        .iter()
        .find(|f| f.beta == beta && f.widths == Some(widths))
    {
        return Some(f);
    }
    let w: &'static [u8] = Box::leak(widths.to_vec().into_boxed_slice());
    let f: &'static GaussianFactory = Box::leak(Box::new(GaussianFactory {
        beta,
        widths: Some(w),
    }));
    cache.push(f);
    Some(f)
}

/// The angle scale of a `Givens` on modes `(p, q)` under `widths` (spec/SPEC-SA.md section 13):
/// `widths[j]` at chain position `j` (modes `2 j + s`, `2 j + 2 + s`), `beta` otherwise.
#[must_use]
pub fn givens_scale(beta: u8, widths: Option<&[u8]>, p: usize, q: usize) -> u8 {
    match widths {
        Some(w) if q == p + 2 => w.get(p / 2).copied().unwrap_or(beta),
        _ => beta,
    }
}

static FACTORIES: [GaussianFactory; 33] = factories();

/// The tracker for `beta`-bit angles, `3 <= beta <= 32`.
#[must_use]
pub fn factory(beta: u32) -> Option<&'static GaussianFactory> {
    (3..=32).contains(&beta).then(|| &FACTORIES[beta as usize])
}

/// Toffolis charged per executed `Givens`: two single-qubit rotations, each `beta - 2` Toffolis
/// by addition into a `beta`-qubit phase-gradient register (Lee et al. 2021, App. C; see
/// spec/DESIGN.md section 15 for the quotes).
#[must_use]
pub fn givens_cost(beta: u8) -> f64 {
    2.0 * f64::from(beta.saturating_sub(2))
}

impl TrackerFactory for GaussianFactory {
    fn lane(&self, system_qubits: usize) -> Box<dyn LaneTracker> {
        let mut l = GaussianLane::new(system_qubits, self.beta);
        l.widths = self.widths;
        Box::new(l)
    }
    /// Tapered specs only: `2 (w - 2)` at the `Givens`'s scale `w` (spec/SPEC-SA.md section 13).
    fn givens_charge(&self, p: usize, q: usize) -> Option<u64> {
        self.widths
            .map(|w| 2 * u64::from(givens_scale(self.beta, Some(w), p, q).saturating_sub(2)))
    }
    fn givens_toffoli_cost(&self) -> f64 {
        givens_cost(self.beta)
    }
    fn phase_gradient_qubits(&self) -> u64 {
        u64::from(self.beta)
    }
    fn quarter_turn(&self) -> Option<u64> {
        (self.beta >= 2).then(|| 1u64 << (self.beta - 2))
    }
    fn as_gaussian(&self) -> Option<&GaussianFactory> {
        Some(self)
    }
}

impl GaussianFactory {
    /// The rotation precision (read by the fast engine, `crate::fastsim`).
    #[must_use]
    pub fn beta(&self) -> u8 {
        self.beta
    }
    /// The tapered schedule, if any (read by the fast engine, `crate::fastsim`).
    #[must_use]
    pub fn widths(&self) -> Option<&'static [u8]> {
        self.widths
    }
}

/// One lane's tracked state.
pub struct GaussianLane {
    n: usize,
    beta: u8,
    /// Tapered widths (spec/SPEC-SA.md section 13); `None`: every rotation at `beta`.
    widths: Option<&'static [u8]>,
    /// Row-major `n x n` mode rotation of `P`; `None` is the identity.
    u: Option<Vec<f64>>,
    /// Frame vectors in time order; `chunks[j]` is where frame `j` starts.
    vecs: Vec<Tv>,
    chunks: Vec<usize>,
    /// `Z/8` phase collected from converting frames to Majorana products.
    phase: u8,
}

impl GaussianLane {
    #[must_use]
    pub fn new(n: usize, beta: u8) -> Self {
        Self {
            n,
            beta,
            widths: None,
            u: None,
            vecs: Vec::new(),
            chunks: Vec::new(),
            phase: 0,
        }
    }

    /// Sets the tapered schedule (as `GaussianFactory::lane` does); used by the fast engine's
    /// self-check to build a reference lane.
    pub fn set_widths(&mut self, widths: Option<&'static [u8]>) {
        self.widths = widths;
    }

    fn push_frame(&mut self, f: &LaneFrame) -> Result<(), String> {
        let (ph, ms) = frame_to_majoranas(f, self.n)?;
        self.phase = (self.phase + ph) % 8;
        if ms.is_empty() {
            return Ok(());
        }
        if self.vecs.len() + ms.len() > MAX_FACTORS {
            return Err(format!(
                "tracker: more than {MAX_FACTORS} Majorana factors on one lane (not supported)"
            ));
        }
        self.chunks.push(self.vecs.len());
        self.vecs
            .extend(ms.iter().map(|&m| Tv::basis(usize::from(m), self.n)));
        Ok(())
    }

    /// The factors of `X` in product order: latest frame first, each frame in its own order.
    fn x_factors(&self) -> Vec<&Tv> {
        let mut out = Vec::with_capacity(self.vecs.len());
        let mut end = self.vecs.len();
        for &start in self.chunks.iter().rev() {
            out.extend(&self.vecs[start..end]);
            end = start;
        }
        out
    }
}

impl LaneTracker for GaussianLane {
    fn givens(&mut self, before: &LaneFrame, p: usize, angle: u64) -> Result<(), String> {
        self.givens_modes(before, p, p + 1, angle)
    }

    fn givens_modes(
        &mut self,
        before: &LaneFrame,
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
        self.push_frame(before)?;
        // A tapered position reads the top `w` bits of the angle (spec/SPEC-SA.md section 13).
        let w = givens_scale(self.beta, self.widths, p, q);
        let angle = if w < self.beta {
            (angle & ((1u64 << w) - 1)) << (self.beta - w)
        } else {
            angle
        };
        let (c, s) = cos_sin(angle, self.beta);
        let n = self.n;
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
        for t in &mut self.vecs {
            rotate(&mut t.v, p, q, c, s);
        }
        Ok(())
    }

    fn finish(&mut self, after: &LaneFrame, reference: Option<&SystemOp>) -> Result<(), String> {
        let (off, c) = self.measure(after, reference)?;
        if off > AD_TOL {
            return Err(format!(
                "the rotated operator differs from the reference: max |Ad(O_ref^-1 O) - I| = {off:.3e} (tolerance {AD_TOL:e})"
            ));
        }
        match c.nearest_w() {
            (0, d) if d <= PHASE_TOL => Ok(()),
            (4, d) if d <= PHASE_TOL => {
                Err("sign flipped: the operator is -1 times the reference".into())
            }
            (j, d) if d <= PHASE_TOL => Err(format!("phase off by w^{j} from the reference")),
            (_, d) => Err(format!("vacuum overlap {c:?} is {d:.3e} from any w^j")),
        }
    }
}

impl GaussianLane {
    /// Folds in the final frame and returns `(max |Ad(Omega) - I|, <vac|Omega|vac>)` for
    /// `Omega = O_ref^-1 O` (`finish` accepts iff the first is at most `AD_TOL` and the second is
    /// within `PHASE_TOL` of 1).
    ///
    /// # Errors
    /// An unsupported frame or a malformed reference.
    pub fn measure(
        &mut self,
        after: &LaneFrame,
        reference: Option<&SystemOp>,
    ) -> Result<(f64, pfaffian::C64), String> {
        self.push_frame(after)?;
        let (ref_phase, refs) = reference_vectors(reference, self.n)?;
        // Omega = O_ref^-1 O = w^k (u_last ... u_first) X P.
        let mut ys: Vec<&Tv> = refs.iter().rev().collect();
        ys.extend(self.x_factors());
        let k = (8 + self.phase - ref_phase % 8) % 8;
        let off = ad_residual(self.u.as_deref(), &ys, self.n);
        Ok((off, pfaffian::vacuum_expectation(&ys).scale_w(k)))
    }
}

/// `max |Ad(Y) (U (+) U) e_b - e_b|` over every basis vector `e_b` of both blocks.
fn ad_residual(u: Option<&[f64]>, ys: &[&Tv], n: usize) -> f64 {
    let mut worst = 0.0f64;
    for block in [false, true] {
        let flip = ys.iter().filter(|y| y.odd != block).count() % 2 == 1;
        let same: Vec<&[f64]> = ys
            .iter()
            .filter(|y| y.odd == block)
            .map(|y| y.v.as_slice())
            .collect();
        let mut y = vec![0.0; n];
        for j in 0..n {
            match u {
                Some(u) => (0..n).for_each(|r| y[r] = u[r * n + j]),
                None => (0..n).for_each(|r| y[r] = f64::from(u8::from(r == j))),
            }
            for w in same.iter().rev() {
                let d = 2.0 * w.iter().zip(&y).map(|(a, b)| a * b).sum::<f64>();
                y.iter_mut()
                    .zip(w.iter())
                    .for_each(|(a, b)| *a = d * b - *a);
            }
            let sign = if flip { -1.0 } else { 1.0 };
            for (r, v) in y.iter().enumerate() {
                let e = (sign * v - f64::from(u8::from(r == j))).abs();
                if e.is_nan() {
                    return f64::INFINITY;
                }
                worst = worst.max(e);
            }
        }
    }
    worst
}

#[cfg(test)]
mod taper_tests {
    //! The tapered tracker (spec/SPEC-SA.md section 13) at small widths, exhaustively.
    use super::*;

    fn identity(n: usize) -> LaneFrame {
        LaneFrame {
            x: vec![0; n.div_ceil(64)],
            z: vec![0; n.div_ceil(64)],
            s_pow: vec![0; n],
            phase: 0,
        }
    }

    /// Mode rotation after one `Givens` on `(p, q)` with register value `a`.
    fn after(f: &GaussianFactory, n: usize, p: usize, q: usize, a: u64) -> Vec<f64> {
        let mut l = GaussianLane::new(n, f.beta);
        l.widths = f.widths;
        l.givens_modes(&identity(n), p, q, a).unwrap();
        l.u.unwrap()
    }

    #[test]
    fn untapered_factories_charge_nothing_extra() {
        for beta in 3..=32 {
            let f = factory(beta).unwrap();
            assert_eq!(f.givens_charge(0, 2), None);
            assert_eq!(f.givens_charge(3, 4), None);
            assert_eq!(f.phase_gradient_qubits(), u64::from(beta));
        }
    }

    /// Every register value at every position of a small schedule: a tapered position `j` (modes
    /// `2 j + s`, `2 j + 2 + s`) rotates exactly as the plain tracker does at the angle's top bits,
    /// `a << (beta - w_j)`; every other mode pair keeps `beta`; the charge is `2 (w - 2)`.
    #[test]
    fn tapered_positions_read_the_top_bits() {
        let widths = [6u8, 5, 4, 3];
        let beta = 6u8;
        let n = 2 * (widths.len() + 1);
        let t = tapered_factory(beta, &widths).unwrap();
        let plain = factory(u32::from(beta)).unwrap();
        assert!(
            std::ptr::eq(t, tapered_factory(beta, &widths).unwrap()),
            "cached"
        );
        for (j, &w) in widths.iter().enumerate() {
            for s in 0..2 {
                let (p, q) = (2 * j + s, 2 * j + 2 + s);
                assert_eq!(t.givens_charge(p, q), Some(2 * u64::from(w - 2)));
                for a in 0..1u64 << w {
                    let got = after(t, n, p, q, a);
                    let want = after(plain, n, p, q, a << (beta - w));
                    assert_eq!(got, want, "position {j} spin {s} value {a}");
                    // Bits above the width are ignored, as the plain tracker ignores bits above beta.
                    assert_eq!(after(t, n, p, q, a | 1 << w), want);
                }
            }
        }
        // Not a chain position: the SpinSwap quarter turn (2 p, 2 p + 1) and any other pair.
        for (p, q) in [(0usize, 1usize), (2, 3), (0, 4), (1, 5)] {
            assert_eq!(t.givens_charge(p, q), Some(2 * u64::from(beta - 2)));
            for a in 0..1u64 << beta {
                assert_eq!(after(t, n, p, q, a), after(plain, n, p, q, a));
            }
        }
        assert_eq!(t.quarter_turn(), plain.quarter_turn());
        assert_eq!(t.phase_gradient_qubits(), u64::from(beta));
    }

    #[test]
    fn schedules_out_of_range_are_refused() {
        assert!(tapered_factory(6, &[6, 2]).is_none());
        assert!(tapered_factory(6, &[7, 6]).is_none());
        assert!(tapered_factory(33, &[6]).is_none());
    }
}
