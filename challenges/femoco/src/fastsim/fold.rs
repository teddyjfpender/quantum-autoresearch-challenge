//! The folding tier with a guard band (spec/FAST-EVALUATOR.md sections 6 and 6.1).
//! Used only for the `Ad` residual half of the final check.
//!
//! **Fold.** A frame handed to the tracker, as `frame_to_majoranas` writes it
//! (`w^k gamma_m1 ... gamma_md`, sorted), equals `w^(k + 2 #pairs) M Z_S`: every pair
//! `gamma_2q gamma_2q+1 = i Z_q` is adjacent and commutes with every other Majorana, and `M` is
//! the product of the unpaired ones in order. `Z_S` is a number-conserving Gaussian that fixes
//! the vacuum (`Ad(Z_S) = D_S`, the sign flip of the modes in `S`, on both Majorana blocks).
//! Pushing it through the tracked state `X P` gives `M (Z_S X Z_S^-1) (Z_S P)`: flip the `S`
//! components of every held vector and the `S` rows of the mode rotation. Both are exact float
//! operations. Only the unpaired Majoranas become vectors (about 2 per lane on the SA circuits,
//! against about `2n` for the reference).
//!
//! **Same ideal operator.** Let `G*_t = G_t / rho_t`, `rho_t = sqrt(c_t^2 + s_t^2)`, be the exact
//! rotation of the *computed* `(c_t, s_t)` (the same floats in both engines). In exact
//! arithmetic with the `G*_t`, the reference's pair vectors `w_q = P* e_q` give, per block,
//! `(2 w w^T - I)(-I) = P* D_q P*^T`, so the reference's `Ad(Omega)` and the fold's are the same
//! matrix `M*_b` on each block `b` (and both use the same reference-operator vectors).
//!
//! **Guard.** `off = max_{b,j,r} |R_b[r][j]|` with `R_b = sigma_b Y_b U - I` computed in floats.
//! For each engine, `gap` bounds `max_{b,j} || computed column j of sigma_b Y_b U - M*_b e_j ||_2`
//! rigorously (module functions below; the derivation is spec/FAST-EVALUATOR.md section 6.1):
//!
//! - the chain's rounding (`2 gamma_k` per dot product of `k` live terms, `(2u + u^2) |d w_r| +
//!   u |y_r|` per update), propagated through the exact reflections;
//! - the inputs' distance from the ideal: per tracked vector a running bound `eps` (each
//!   rotation adds `kappa_t (|a| + |b|)` for its two touched components `a, b`, with `kappa_t =
//!   sqrt(2) (2u + u^2) rho_t + |rho_t - 1|`), the mode rotation's a-priori bound, and per group
//!   of vectors that are columns of one computed rotation product (a chunk) the compact-WY
//!   bound on the product of their reflections.
//!
//! Then `|off_ref - off_fold| <= gap_ref + gap_fold =: g`. The fold's residual is used only when
//! `off_fold + g <= AD_TOL` (with outward rounding), which proves `off_ref <= AD_TOL`; otherwise
//! the caller computes the reference residual exactly (a counted fallback). A fold that does not
//! certify can never change a verdict, and a certificate is only issued when the reference's
//! residual is provably below the tolerance, so verdicts are identical on every input.
use crate::sim::gaussian::{cos_sin, frame_to_majoranas, givens_scale, reference_vectors, Tv};
use crate::sim::tracker::LaneFrame;
use crate::spec::SystemOp;

/// Unit roundoff of `f64` (round to nearest): `2^-53`.
pub const U: f64 = f64::EPSILON / 2.0;

/// Outward-rounding factor for every bound computed in floats here. Each bound is a sum or
/// product of at most about `10^6` non-negative terms, each computed with relative error at most
/// a few `u`, so the computed value is at least `(1 - 10^-9)` times the exact one; multiplying
/// by `1 + 10^-6` makes it an upper bound.
const OUT: f64 = 1.0 + 1e-6;
/// Absolute slack for gradual underflow: each float operation adds at most `2^-1074` beyond the
/// relative model; far fewer than `10^20` operations enter any bound.
const TINY: f64 = 1e-300;

/// `kappa` for computed `(c, s)`: with `nu >= |c^2 + s^2 - 1| >= |rho - 1|`, `kappa = (2u + u^2)
/// (1 + nu) + nu`, so one computed rotation of `(a, b)` differs from `G* (a, b)` by at most
/// `kappa (|a| + |b|)` in the 2-norm (docs section 6.1, step 1). `None` when `c^2 + s^2` is far
/// from 1 (never for `cos_sin`).
#[must_use]
pub fn kappa(c: f64, s: f64) -> Option<f64> {
    if !(c.is_finite() && s.is_finite()) {
        return None;
    }
    // c^2 = ph + pl and s^2 = qh + ql exactly (FMA two-product; no underflow at |c|, |s| <= 1
    // unless the value is below 1e-150, where pl, ql are then bounded by TINY instead).
    let ph = c * c;
    let pl = c.mul_add(c, -ph);
    let qh = s * s;
    let ql = s.mul_add(s, -qh);
    let sum = ph + qh; // |sum - (ph + qh)| <= u sum
    if !(0.5..=2.0).contains(&sum) {
        return None;
    }
    // Sterbenz: sum - 1 is exact for sum in [0.5, 2].
    let nu = ((sum - 1.0).abs() + U * sum + pl.abs() + ql.abs() + TINY) * OUT;
    let k = ((2.0 * U + U * U) * (1.0 + nu) + nu) * OUT;
    Some(k)
}

/// `gamma_k = k u / (1 - k u)`.
fn gamma(k: usize) -> f64 {
    let ku = k as f64 * U;
    ku / (1.0 - ku) * OUT
}

/// A small bit set over `n` rows.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }
    fn of(v: &[f64]) -> Self {
        let mut b = Self::new(v.len());
        for (i, &x) in v.iter().enumerate() {
            if x != 0.0 {
                b.set(i);
            }
        }
        b
    }
    fn meet(&self, o: &Self) -> u32 {
        self.0
            .iter()
            .zip(&o.0)
            .map(|(a, b)| (a & b).count_ones())
            .sum()
    }
    fn or(&mut self, o: &Self) {
        self.0.iter_mut().zip(&o.0).for_each(|(a, b)| *a |= b);
    }
}

/// What `gap` knows of the mode rotation `U` the chain starts from.
#[derive(Clone, Copy)]
pub enum Cols<'a> {
    /// No rotation ran: `U` is the identity, exactly.
    Identity,
    /// `U` as computed (row-major `n x n`).
    Matrix(&'a [f64]),
    /// `U` not formed: per row, a superset of the columns whose entry may be non-zero (bit
    /// sets of `ceil(n / 64)` words). Its entries are finite: each rotation maps a column of
    /// norm at most `1 + eps_U` to one of norm at most `(1 + nu)` times that.
    Rows(&'a [u64]),
}

/// One factor of `Y` as the residual sees it: its vector, the running bound on its distance
/// from the ideal vector, and its group (factors of one group are columns of one computed
/// rotation product applied to distinct basis vectors).
pub struct Factor<'a> {
    pub y: &'a Tv,
    pub eps: f64,
    pub group: usize,
}

/// Rigorous bound on `max_{b,j} || computed (sigma_b Y_b U e_j) - M*_b e_j ||_2` for the
/// residual `kernels::ad_residual(u, ys)` (the reference's float operations), where `ys` are
/// the factors in product order, `cols` describes the mode rotation and `eps_u` bounds
/// `|| (U - U*) e_j ||_2` for every `j`. `None` when no useful bound exists (a
/// non-finite input or a factor too far from the ideal); the caller then falls back.
#[must_use]
pub fn gap(cols: Cols<'_>, eps_u: f64, ys: &[Factor<'_>], n: usize) -> Option<f64> {
    let u = match cols {
        Cols::Matrix(u) => Some(u),
        _ => None,
    };
    if !eps_u.is_finite()
        || u.is_some_and(|u| u.len() != n * n || !u.iter().all(|x| x.is_finite()))
        || ys
            .iter()
            .any(|f| f.y.v.len() != n || !f.eps.is_finite() || !f.y.v.iter().all(|x| x.is_finite()))
    {
        return None;
    }
    let groups = ys.iter().map(|f| f.group + 1).max().unwrap_or(0);
    // Column supports of U as computed (exact zeros are exact in every later operation).
    let mut col_supp: Vec<Bits> = Vec::with_capacity(n);
    let words = n.div_ceil(64);
    for j in 0..n {
        let mut b = Bits::new(n);
        match cols {
            Cols::Matrix(u) => (0..n)
                .filter(|&r| u[r * n + j] != 0.0)
                .for_each(|r| b.set(r)),
            Cols::Rows(rows) => (0..n)
                .filter(|&r| rows[r * words + j / 64] >> (j % 64) & 1 == 1)
                .for_each(|r| b.set(r)),
            Cols::Identity => b.set(j),
        }
        col_supp.push(b);
    }
    if let Cols::Rows(rows) = cols {
        if rows.len() != n * words {
            return None;
        }
    }
    let mut worst = 0.0f64;
    for block in [false, true] {
        // Input term: || Y_b - Y*_b || <= prod_g (1 + err_g) - 1.
        let mut prod = 1.0f64;
        for g in 0..groups {
            let es: Vec<f64> = ys
                .iter()
                .filter(|f| f.group == g && f.y.odd == block)
                .map(|f| f.eps)
                .collect();
            if es.is_empty() {
                continue;
            }
            let phi = (es.iter().map(|e| e * e).sum::<f64>().sqrt() + TINY) * OUT;
            let e = if es.len() > 1 {
                (2.0 * phi + phi * phi) / std::f64::consts::SQRT_2 * OUT
            } else {
                0.0
            };
            if 2.0 * e >= 0.5 || phi >= 0.25 {
                return None;
            }
            let err = 2.0
                * ((1.0 + phi) * (1.0 + phi) * 2.0 * e / (1.0 - 2.0 * e) + 2.0 * phi + phi * phi)
                * OUT;
            prod *= 1.0 + err;
        }
        let t_b = (prod - 1.0) * OUT;
        let b_in = ((1.0 + t_b) * (1.0 + eps_u) - 1.0) * OUT;
        // Chain term: the reference's reflections in application order (`same.iter().rev()`).
        let steps: Vec<(Bits, f64)> = ys
            .iter()
            .rev()
            .filter(|f| f.y.odd == block)
            .map(|f| (Bits::of(&f.y.v), f.eps))
            .collect();
        let mut seen: std::collections::HashMap<&Bits, f64> = std::collections::HashMap::new();
        let mut a_max = 0.0f64;
        for start in &col_supp {
            if let Some(&a) = seen.get(start) {
                a_max = a_max.max(a);
                continue;
            }
            let mut supp = start.clone();
            let mut ny = 1.0 + eps_u; // >= || exact chain so far ||
            let mut err = 0.0f64; // >= || computed - exact chain so far ||
            for (sw, ew) in &steps {
                let k = supp.meet(sw) as usize;
                if k == 0 {
                    // Every product is an exact zero: y -> -y exactly, in both chains.
                    continue;
                }
                let w2 = (1.0 + ew) * (1.0 + ew);
                let eta = 1.0 + 2.0 * (w2 - 1.0);
                let gk = gamma(k);
                let lambda = 2.0 * gk * w2 + 2.0 * (2.0 * U + U * U) * (1.0 + gk) * w2 + U;
                err = (eta * err + lambda * (ny + err) + TINY) * OUT;
                ny *= eta * OUT;
                supp.or(sw);
            }
            seen.insert(start, err);
            a_max = a_max.max(err);
        }
        worst = worst.max((a_max + b_in) * OUT);
    }
    worst.is_finite().then_some(worst)
}

/// `(phase, vectors, (bound, group) per vector)` of a reference operator.
pub type RefFactors = (u8, Vec<Tv>, Vec<(f64, usize)>);

/// The reference operator's vectors (`reference_vectors`, unchanged) with, per vector, its
/// running bound and its group: a `Rotated` part rotates distinct basis vectors by one network's
/// rotations, so its vectors are columns of one computed product (one group per part); a
/// monomial's vectors are exact basis vectors (bound 0).
///
/// # Errors
/// As `reference_vectors`.
pub fn reference_factors(r: Option<&SystemOp>, n: usize) -> Result<RefFactors, String> {
    let (k, vs) = reference_vectors(r, n)?;
    let mut bounds = Vec::with_capacity(vs.len());
    if let Some(SystemOp::Rotated(rot)) = r {
        for (pi, part) in rot.parts.iter().enumerate() {
            let beta = part.network.beta;
            let map = |p: u16| match part.spin {
                Some(s) => 2 * usize::from(p) + usize::from(s),
                None => usize::from(p),
            };
            let rots: Vec<(usize, usize, f64, f64, f64)> = part
                .network
                .rotations
                .iter()
                .map(|&(p, q, a)| {
                    let (c, s) = cos_sin(u64::from(a), beta);
                    (map(p), map(q), c, s, kappa(c, s).unwrap_or(f64::INFINITY))
                })
                .collect();
            for &m in &part.majoranas {
                // The reference's rotations replayed (same floats) for the touched magnitudes.
                let mut v = Tv::basis(usize::from(m), n).v;
                let mut eps = 0.0f64;
                for &(p, q, c, s, kap) in &rots {
                    let (x, y) = (v[p], v[q]);
                    eps += kap * (x.abs() + y.abs());
                    v[p] = c * x - s * y;
                    v[q] = s * x + c * y;
                }
                bounds.push((eps * OUT, pi));
            }
        }
    } else {
        bounds.extend((0..vs.len()).map(|i| (0.0, i)));
    }
    if bounds.len() != vs.len() {
        return Err("reference op: vector count mismatch".into());
    }
    Ok((k, vs, bounds))
}

/// The fold tracker: a mode rotation with the `Z` strings folded in, and the unpaired
/// Majoranas as vectors (with their running bounds). Rotations are the reference's
/// (`givens_modes`): same angle handling, same float operations on rows and vectors.
pub struct FoldLane {
    n: usize,
    beta: u8,
    widths: Option<&'static [u8]>,
    pub u: Option<Vec<f64>>,
    pub vecs: Vec<Tv>,
    pub eps: Vec<f64>,
    chunks: Vec<usize>,
    /// A-priori bound on `|| (U - U*) e_j ||_2`.
    pub eps_u: f64,
}

impl FoldLane {
    #[must_use]
    pub fn new(n: usize, beta: u8, widths: Option<&'static [u8]>) -> Self {
        Self {
            n,
            beta,
            widths,
            u: None,
            vecs: Vec::new(),
            eps: Vec::new(),
            chunks: Vec::new(),
            eps_u: 0.0,
        }
    }

    /// Hands a frame to the tracker, folded.
    ///
    /// # Errors
    /// As `frame_to_majoranas` (the reference fails the same way first).
    pub fn push_frame(&mut self, f: &LaneFrame) -> Result<(), String> {
        let (_, ms) = frame_to_majoranas(f, self.n)?;
        self.push_ms(&ms);
        Ok(())
    }

    /// Hands over a frame given as its sorted Majoranas (`frame_to_majoranas`), folded.
    pub fn push_ms(&mut self, ms: &[u16]) {
        let mut unpaired = Vec::with_capacity(ms.len());
        let mut flips = Vec::new();
        let mut i = 0;
        while i < ms.len() {
            let m = usize::from(ms[i]);
            if m % 2 == 0 && i + 1 < ms.len() && usize::from(ms[i + 1]) == m + 1 {
                flips.push(m / 2);
                i += 2;
            } else {
                unpaired.push(m);
                i += 1;
            }
        }
        if !flips.is_empty() {
            let n = self.n;
            let u = self.u.get_or_insert_with(|| {
                let mut m = vec![0.0; n * n];
                (0..n).for_each(|i| m[i * n + i] = 1.0);
                m
            });
            for &q in &flips {
                u[q * n..(q + 1) * n].iter_mut().for_each(|x| *x = -*x);
                for t in &mut self.vecs {
                    t.v[q] = -t.v[q];
                }
            }
        }
        if !unpaired.is_empty() {
            self.chunks.push(self.vecs.len());
            for m in unpaired {
                self.vecs.push(Tv::basis(m, self.n));
                self.eps.push(0.0);
            }
        }
    }

    /// `givens_modes` with the frame given as its Majoranas (`push_ms`).
    pub fn givens_modes_ms(&mut self, before: Option<&[u16]>, p: usize, q: usize, angle: u64) {
        if let Some(ms) = before {
            self.push_ms(ms);
        }
        self.rotate(p, q, angle);
    }

    /// A `Givens` after the frame (`None`: the identity frame). The caller has already run the
    /// reference-equivalent checks (`p < q < n`).
    ///
    /// # Errors
    /// As `push_frame`.
    pub fn givens_modes(
        &mut self,
        before: Option<&LaneFrame>,
        p: usize,
        q: usize,
        angle: u64,
    ) -> Result<(), String> {
        if let Some(f) = before {
            self.push_frame(f)?;
        }
        self.rotate(p, q, angle);
        Ok(())
    }

    /// The rotation of a `Givens` (the reference's angle handling and float operations).
    fn rotate(&mut self, p: usize, q: usize, angle: u64) {
        let w = givens_scale(self.beta, self.widths, p, q);
        let angle = if w < self.beta {
            (angle & ((1u64 << w) - 1)) << (self.beta - w)
        } else {
            angle
        };
        let (c, s) = cos_sin(angle, self.beta);
        let kap = kappa(c, s).unwrap_or(f64::INFINITY);
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
        self.eps_u = (self.eps_u + std::f64::consts::SQRT_2 * kap * (1.0 + self.eps_u)) * OUT;
        for (t, e) in self.vecs.iter_mut().zip(&mut self.eps) {
            let (a, b) = (t.v[p], t.v[q]);
            if a != 0.0 || b != 0.0 {
                *e += kap * (a.abs() + b.abs());
                t.v[p] = c * a - s * b;
                t.v[q] = s * a + c * b;
            }
        }
    }

    /// The factors of `X` in product order (latest frame first), with group ids by chunk.
    #[must_use]
    pub fn x_factors(&self, first_group: usize) -> Vec<Factor<'_>> {
        let mut out = Vec::with_capacity(self.vecs.len());
        let mut end = self.vecs.len();
        for (g, &start) in self.chunks.iter().enumerate().rev() {
            for i in start..end {
                out.push(Factor {
                    y: &self.vecs[i],
                    eps: self.eps[i] * OUT,
                    group: first_group + g,
                });
            }
            end = start;
        }
        out
    }
}

/// The guard's decision: `(off_fold + g) (1 + 8u) <= AD_TOL`. Each engine's computed `off` is
/// within a relative `u` of the exact maximum residual of its computed chain (the final
/// `|sigma v - delta|` rounds once), and those two exact maxima differ by at most `g`, so this
/// proves the reference's computed residual is at most `AD_TOL`. Non-finite values never
/// certify.
#[must_use]
pub fn certifies(off_fold: f64, g: Option<f64>) -> bool {
    g.is_some_and(|g| {
        off_fold.is_finite()
            && g.is_finite()
            && (off_fold + g) * (1.0 + 8.0 * U) <= crate::sim::gaussian::AD_TOL
    })
}

/// The guard's decisions in one run (`Stats::guard`; stderr with `FEMOCO_FASTSIM_STATS=1` and
/// the equivalence harness's counters; never in `score.json`).
#[derive(Default)]
pub struct GuardCounts {
    /// Residuals the guard certified (the exact residual was not computed).
    pub certified: std::sync::atomic::AtomicU64,
    /// Fold tried, guard declined: the exact residual decided.
    pub fallback: std::sync::atomic::AtomicU64,
    /// Too few vectors to fold: the exact residual decided.
    pub small: std::sync::atomic::AtomicU64,
    /// Largest `g` seen, as bits.
    pub max_g: std::sync::atomic::AtomicU64,
}

fn fetch_max(a: &std::sync::atomic::AtomicU64, v: f64) {
    if v.is_finite() && v >= 0.0 {
        a.fetch_max(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// Records one guard decision.
pub fn note(counts: &GuardCounts, certified: bool, g: Option<f64>) {
    use std::sync::atomic::Ordering;
    if certified {
        counts.certified.fetch_add(1, Ordering::Relaxed);
    } else {
        counts.fallback.fetch_add(1, Ordering::Relaxed);
    }
    fetch_max(&counts.max_g, g.unwrap_or(f64::INFINITY));
}

/// Largest `|off_ref - off_fold| / g` seen by the self-check (process-wide, tooling).
static MAX_RATIO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Records a self-check ratio `|off_ref - off_fold| / g`.
pub fn note_ratio(r: f64) {
    fetch_max(&MAX_RATIO, r);
}

/// The largest self-check ratio since the last call.
pub fn take_ratio() -> f64 {
    f64::from_bits(MAX_RATIO.swap(0, std::sync::atomic::Ordering::Relaxed))
}
