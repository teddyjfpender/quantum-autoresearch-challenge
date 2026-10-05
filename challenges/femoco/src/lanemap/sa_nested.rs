//! Lane-map family `sa-nested-alias-v1`: the spectrum-amplified block encoding of Low et al. 2025
//! (Phys. Rev. X 15, 041016, Eq. (10) and Fig. 2) over an `sos-sa` spec (spec/SPEC-SA.md section 5).
//!
//!
//! Uniform register, low bits first: outer alias index `k_o`, outer keep `mu_o`, the outer spin bit
//! `s1`; then the inner register: inner alias index `k_i`, inner keep `mu_i`, the inner spin bit
//! `s0`. So `u_o = k_o + mu_o + 1`, `w = k_i + mu_i + 1`, `u = u_o + w <= 63`.
//!
//! - The outer alias (alias-v1 on bits `0..k_o + mu_o`) names an outer item: one-body eigenvector
//!   `r < N`, whose generator is `(r, s1)`; or square `(r, c)` at `N + r C + c`, which ignores `s1`.
//! - The item's inner alias (bits `u_o .. u_o + k_i + mu_i`, one table per outer item, one shared
//!   width) names an inner item: `x in {0, 1}` for a one-body generator (`M_x`, ignoring `s0`), or
//!   `j in 0..=B` for a square (`j < B` is `M_(j, s0)`, `j = B` is the identity).
//!
//! A lane crosses the one `Reflect` (spec/DESIGN.md section 16): the inner register holds `a`
//! before it and `b` after. The reference a control-1 lane must apply is `M(b)^dagger M(a)`, the
//! second pass on the left, so that the step block-encodes
//! `Lambda_decl sum_alpha p_alpha (2 O_alpha^dagger O_alpha / lambda_alpha^2 - 1)` (Low et al. Eq. (10)).
//! A second copy that is identical op for op can apply `M^dagger` where `M` is not Hermitian (the
//! one-body `i gamma`) by reading a pass flag the first copy leaves, as the nested DF folded
//! layout does.
//!
//! Counts are exact alias counts and `rounding_error` is `sum_t |c_t - c^_t|` over the spec's
//! ordered products `t = (alpha, x, y)`, `x != y`, in dyadic integers.
//!
//! Payload: `u32 k_o, mu_o, k_i, mu_i`, `lambda_decl` (`Exact::to_bytes`), `keep_o[2^k_o],
//! alt_o[2^k_o]`, then `N + R C` inner tables `keep[2^k_i], alt[2^k_i]` (u32 little-endian).
use super::alias::{MAX_K, MAX_LAMBDA_EXPONENT};
use super::df_nested::Table;
use super::LaneMap;
use crate::spec::rounding::{self, Estimate, EstimatedParams};
use crate::spec::sa::{adjoint, product, Generator, SaSpec};
use crate::spec::{EncodingSpec, Exact, Monomial, SystemOp};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

pub const FAMILY: &str = "sa-nested-alias-v1";

mod ground;
pub use ground::{GroundBound, RULE as GROUND_RULE};

pub struct SaNestedMap {
    pub lambda_decl: Exact,
    pub outer: Table,
    /// One table per outer item (`N + R C`), all of one width.
    pub inner: Vec<Table>,
    n: usize,
    r: usize,
    b: usize,
    c: usize,
}

fn sa(spec: &dyn EncodingSpec) -> Result<&SaSpec, String> {
    spec.as_any()
        .downcast_ref::<SaSpec>()
        .ok_or_else(|| format!("{FAMILY}: spec {} is not an sos-sa spec", spec.id()))
}

/// `m / 2^e` without gcd reduction.
struct Dy {
    m: BigInt,
    e: u32,
}

impl Dy {
    fn of(x: &Exact) -> Result<Self, String> {
        let e = x
            .dyadic_exponent()
            .ok_or(format!("{FAMILY}: value is not dyadic"))?;
        Ok(Self {
            m: x.num.clone(),
            e,
        })
    }
    fn abs_f64(x: f64) -> Result<Self, String> {
        Self::of(&Exact::from_f64(x)?.abs())
    }
}

/// The alias-v1 table rules (spec/DESIGN.md section 6): `2^k` entries, every alt an item, every
/// padding bucket `keep = 0`, every keep below `2^mu`.
fn check_table(t: &Table, items: usize, what: &str) -> Result<(), String> {
    let buckets = 1usize << t.k;
    if t.keep.len() != buckets || t.alt.len() != buckets {
        return Err(format!(
            "{FAMILY}: {what} table needs 2^k = {buckets} entries"
        ));
    }
    if items > buckets {
        return Err(format!(
            "{FAMILY}: {what} has {items} items but 2^k = {buckets} buckets"
        ));
    }
    if let Some(i) = t.alt.iter().position(|&a| a as usize >= items) {
        return Err(format!(
            "{FAMILY}: {what} alt[{i}] = {} is not an item ({items} items)",
            t.alt[i]
        ));
    }
    if let Some(i) = (items..buckets).find(|&i| t.keep[i] != 0) {
        return Err(format!(
            "{FAMILY}: {what} bucket {i} is not an item and must have keep = 0"
        ));
    }
    if let Some(i) = t.keep.iter().position(|&v| u64::from(v) >> t.mu != 0) {
        return Err(format!("{FAMILY}: {what} keep[{i}] is not below 2^mu"));
    }
    Ok(())
}

impl SaNestedMap {
    /// Checks the tables against the spec (spec/SPEC-SA.md section 5).
    ///
    /// # Errors
    /// Widths out of range, `u > 63`, inner tables of different widths or the wrong count, an
    /// alias target that is not an item, a padding bucket that keeps lanes, or a `lambda_decl`
    /// that is not positive and dyadic.
    pub fn new(
        lambda_decl: Exact,
        outer: Table,
        inner: Vec<Table>,
        spec: &SaSpec,
    ) -> Result<Self, String> {
        let first = inner.first().ok_or(format!("{FAMILY}: no inner tables"))?;
        let (k_i, mu_i) = (first.k, first.mu);
        if outer.k > MAX_K || k_i > MAX_K || outer.mu > 31 || mu_i > 31 {
            return Err(format!(
                "{FAMILY}: widths ({}, {}, {k_i}, {mu_i}) out of range",
                outer.k, outer.mu
            ));
        }
        let u = outer.k + outer.mu + 1 + k_i + mu_i + 1;
        if u > 63 {
            return Err(format!(
                "{FAMILY}: u = k_o + mu_o + 1 + k_i + mu_i + 1 = {u} above 63"
            ));
        }
        if let Some(t) = inner.iter().position(|t| (t.k, t.mu) != (k_i, mu_i)) {
            return Err(format!(
                "{FAMILY}: inner table {t} has (k, mu) = ({}, {}) but table 0 has ({k_i}, {mu_i}); \
                 every inner table must share one width",
                inner[t].k, inner[t].mu
            ));
        }
        let items = spec.outer_items();
        if inner.len() != items {
            return Err(format!(
                "{FAMILY}: {} inner tables, the spec needs {items}",
                inner.len()
            ));
        }
        check_table(&outer, items, "outer")?;
        for (o, t) in inner.iter().enumerate() {
            check_table(t, spec.inner_items(o), &format!("inner table {o}"))?;
        }
        if lambda_decl.dyadic_exponent().is_none() || lambda_decl <= Exact::zero() {
            return Err(format!(
                "{FAMILY}: lambda_decl {lambda_decl} must be positive and dyadic"
            ));
        }
        if lambda_decl
            .dyadic_exponent()
            .is_some_and(|e| e > MAX_LAMBDA_EXPONENT)
        {
            return Err(format!(
                "{FAMILY}: lambda_decl exponent above {MAX_LAMBDA_EXPONENT}"
            ));
        }
        Ok(Self {
            lambda_decl,
            outer,
            inner,
            n: spec.n,
            r: spec.r,
            b: spec.b,
            c: spec.c,
        })
    }

    /// Outer bits `u_o = k_o + mu_o + 1`: the inner register starts here.
    #[must_use]
    pub fn outer_bits(&self) -> u32 {
        self.outer.k + self.outer.mu + 1
    }

    /// Inner register width `w = k_i + mu_i + 1`.
    #[must_use]
    pub fn inner_width(&self) -> u32 {
        self.inner[0].k + self.inner[0].mu + 1
    }

    /// The outer alias item lane `s` names (`r < N` one-body, else square `N + r C + c`).
    #[must_use]
    pub fn outer_item(&self, s: u64) -> usize {
        self.outer.item(s)
    }

    /// The generator lane `s` names.
    #[must_use]
    pub fn decode_outer(&self, s: u64) -> Generator {
        let o = self.outer_item(s);
        if o < self.n {
            let spin = ((s >> (self.outer.k + self.outer.mu)) & 1) as u8;
            Generator::OneBody { r: o, spin }
        } else {
            let q = o - self.n;
            Generator::Square {
                r: q / self.c,
                c: q % self.c,
            }
        }
    }

    /// `(inner item, s0)` that inner value `a` names for outer item `o`.
    #[must_use]
    pub fn inner_item(&self, o: usize, a: u64) -> (usize, u8) {
        let t = &self.inner[o];
        (t.item(a), ((a >> (t.k + t.mu)) & 1) as u8)
    }

    fn m_op(&self, d: &SaSpec, o: usize, g: Generator, a: u64) -> SystemOp {
        let (x, s0) = self.inner_item(o, a);
        match g {
            Generator::OneBody { r, spin } => d.one_body_op(r, spin, x),
            Generator::Square { r, c } => d.square_op(r, c, x, s0),
        }
    }

    fn operator(&self, d: &SaSpec, s: u64, after: u64) -> SystemOp {
        let u_o = self.outer_bits();
        let (a, b) = (s >> u_o, after >> u_o);
        let o = self.outer_item(s);
        let g = self.decode_outer(s);
        product(adjoint(&self.m_op(d, o, g, b)), self.m_op(d, o, g, a))
    }
}

impl LaneMap for SaNestedMap {
    fn family(&self) -> &str {
        FAMILY
    }
    fn uniform_bits(&self) -> u32 {
        self.outer_bits() + self.inner_width()
    }
    fn lambda_decl(&self) -> Exact {
        self.lambda_decl.clone()
    }
    /// The diagonal reference (second-pass value equal to the first): the identity.
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp {
        self.reference_nested(spec, s, s)
    }
    fn reference_nested(&self, spec: &dyn EncodingSpec, s: u64, after: u64) -> SystemOp {
        match sa(spec) {
            Ok(d) => self.operator(d, s, after),
            Err(_) => SystemOp::Monomial(Monomial {
                phase: 0,
                majoranas: Vec::new(),
            }),
        }
    }
    fn inner_bits(&self) -> Option<(u32, u32)> {
        Some((self.outer_bits(), self.inner_width()))
    }
    /// `sum_t |c_t - c^_t|` over the ordered products (spec/SPEC-SA.md section 5):
    /// `c^_(alpha,x,y) = 2 lambda_decl n_alpha m_x m_y / 2^(u_o + 2w)` with `n_alpha` the outer lanes
    /// of the generator (`n_r` for one-body `(r, s1)`, `2 n_rc` for a square, both `s1`) and `m_x`
    /// the inner lanes of `x` (`2 m_x` for one-body `x` and for the identity item, `m_b` for
    /// `(b, s0)`), against `c = |e_r| / 4` (one-body), `|w_b w_b'| / 8` (two spin items) and
    /// `|wB w_b| / 4` (identity and spin item).
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String> {
        let d = sa(spec)?;
        if (d.n, d.r, d.b, d.c) != (self.n, self.r, self.b, self.c) {
            return Err(format!("{FAMILY}: lane map was built for a different spec"));
        }
        let (u_o, w) = (self.outer_bits(), self.inner_width());
        let lam = Dy::of(&self.lambda_decl)?;
        let e: Vec<Dy> =
            d.e.iter()
                .map(|&x| Dy::abs_f64(x))
                .collect::<Result<_, _>>()?;
        let wv: Vec<Dy> =
            d.w.iter()
                .map(|&x| Dy::abs_f64(x))
                .collect::<Result<_, _>>()?;
        let wb: Vec<Dy> =
            d.wb.iter()
                .map(|&x| Dy::abs_f64(x))
                .collect::<Result<_, _>>()?;
        // Common exponent: c = e/4 (e.e + 2), w w'/8 (e_w + e_w' + 3), wB w/4 (e_B + e_w + 2);
        // encoded 2 lambda n m m / 2^(u_o + 2w) (lam.e + u_o + 2w).
        let max_e = |v: &[Dy]| v.iter().map(|x| x.e).max().unwrap_or(0);
        let (ee, ew, eb) = (max_e(&e), max_e(&wv), max_e(&wb));
        let ex = (ee + 2)
            .max(2 * ew + 3)
            .max(eb + ew + 2)
            .max(lam.e + u_o + 2 * w);
        let at = |x: &Dy, extra: u32| -> BigInt { &x.m << (ex - extra - x.e) };
        // 2 lambda n m_x m_y / 2^(u_o + 2w) at exponent ex, i.e. lam.m * (2 n m_x m_y) << shift.
        let shift = ex - lam.e - u_o - 2 * w;
        let enc = |nmm: BigInt| (&lam.m * nmm) << shift;
        let mut total = BigInt::zero();
        let outer = self.outer.counts(d.outer_items());
        for (r, er) in e.iter().enumerate() {
            let m = self.inner[r].counts(2);
            // one-body (r, s1) for each s1: n_alpha = outer[r]; inner lanes 2 m_x.
            let c = at(er, 2);
            let got = enc(BigInt::from(2 * outer[r]) * (2 * m[0]) * (2 * m[1]));
            total += (&c - &got).abs() * 4; // 2 spins x 2 orders, all equal
        }
        for q in 0..d.r * d.c {
            let o = d.n + q;
            let n2 = BigInt::from(2 * 2 * outer[o]); // the factor 2 of c^, times n_alpha = 2 n_rc
            let m = self.inner[o].counts(d.b + 1);
            let ws = &wv[q * d.b..(q + 1) * d.b];
            let bb = &wb[q];
            // Spin items (b, s0) x (b', s0'), distinct: lanes m_b each.
            for (i, wi) in ws.iter().enumerate() {
                let ni = &n2 * m[i];
                for (j, wj) in ws.iter().enumerate() {
                    let c = (&wi.m * &wj.m) << (ex - 3 - wi.e - wj.e);
                    let got = enc(&ni * m[j]);
                    // (i, s) x (j, s'): 4 spin combinations, minus (i, s) = (j, s) when i = j.
                    let pairs = if i == j { 2 } else { 4 };
                    total += (&c - &got).abs() * pairs;
                }
                // Identity item (lanes 2 m_B) with (i, s0), both orders, both spins.
                let c = (&bb.m * &wi.m) << (ex - 2 - bb.e - wi.e);
                let got = enc(&ni * (2 * m[d.b]));
                total += (&c - &got).abs() * 4;
            }
        }
        Ok(Exact::dyadic(total, ex))
    }
    /// The estimated class's procedure (spec/SPEC-SA.md section 14, `spec::rounding`): `lambda_decl` must be
    /// the spec's `Lambda` exactly, every count of every table the floor or the ceiling of its ideal
    /// `T w_i / sum w` (exact rationals: `|n_i sum w - T w_i| < sum w`), and the tables' resolution
    /// `b_equiv` is reported for the class's bit rule. Weights are the ones `build` rounds: outer
    /// `2 |e_r|` and `S_rc^2 / 2`, inner `(1, 1)` and `(|w_rc0|, ..., |w_rc,B-1|, |wB_rc|)`.
    fn rounding_estimate(
        &self,
        spec: &dyn EncodingSpec,
        params: &EstimatedParams,
    ) -> Result<Estimate, String> {
        let d = sa(spec)?;
        if (d.n, d.r, d.b, d.c) != (self.n, self.r, self.b, self.c) {
            return Err(format!("{FAMILY}: lane map was built for a different spec"));
        }
        if self.lambda_decl != d.lambda {
            return Err(format!(
                "estimated rounding class: lambda_decl {} must equal the spec's Lambda {} exactly",
                self.lambda_decl, d.lambda
            ));
        }
        let ex = |x: f64| Exact::from_f64(x).map(|v| v.abs());
        let mut outer_w = Vec::with_capacity(d.outer_items());
        for &e in &d.e {
            outer_w.push(ex(e)?.add(&ex(e)?));
        }
        let mut inner_w: Vec<Vec<Exact>> = vec![vec![Exact::from_int(1), Exact::from_int(1)]; d.n];
        for q in 0..d.r * d.c {
            let mut v = d.w[q * d.b..(q + 1) * d.b]
                .iter()
                .map(|&x| ex(x))
                .collect::<Result<Vec<_>, _>>()?;
            v.push(ex(d.wb[q])?);
            let s = v.iter().fold(Exact::zero(), |a, x| a.add(x));
            outer_w.push(s.mul(&s).mul(&Exact::dyadic(1.into(), 1)));
            inner_w.push(v);
        }
        let check = |t: &Table, w: &[Exact], what: &str| -> Result<(), String> {
            let lanes = Exact::from_int(1i64 << (t.k + t.mu));
            let total = w.iter().fold(Exact::zero(), |a, x| a.add(x));
            if total.is_zero() {
                return Err(format!("estimated rounding class: {what} has zero weight"));
            }
            for (i, (n, wi)) in t.counts(w.len()).iter().zip(w).enumerate() {
                let n = Exact::from_int(i64::try_from(*n).map_err(|e| e.to_string())?);
                if n.mul(&total).sub(&lanes.mul(wi)).abs() >= total {
                    return Err(format!(
                        "estimated rounding class: {what} item {i} has {} lanes, not the floor or the \
                         ceiling of its ideal {:.6}",
                        n,
                        lanes.mul(wi).to_f64() / total.to_f64()
                    ));
                }
            }
            Ok(())
        };
        check(&self.outer, &outer_w, "the outer table")?;
        for (o, (t, w)) in self.inner.iter().zip(&inner_w).enumerate() {
            check(t, w, &format!("inner table {o}"))?;
        }
        let (k_i, mu_i) = (self.inner[0].k, self.inner[0].mu);
        Ok(Estimate {
            params: params.clone(),
            outer_bits: (self.outer.k, self.outer.mu),
            inner_bits: (k_i, mu_i),
            b_equiv_outer: rounding::b_equiv(d.outer_items(), self.outer.k + self.outer.mu),
            b_equiv_inner: rounding::b_equiv(d.b + 1, k_i + mu_i),
            beta: u32::from(d.beta),
            tables: 1 + self.inner.len(),
        })
    }
    fn alias_bits(&self) -> Option<(u32, u32)> {
        Some((self.outer.k, self.outer.mu))
    }
    fn ground_bound(
        &self,
        spec: &dyn EncodingSpec,
        g: &Exact,
    ) -> Option<Result<GroundBound, String>> {
        Some(sa(spec).and_then(|d| SaNestedMap::ground_bound(self, d, g)))
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = super::MAGIC.to_vec();
        out.extend_from_slice(&u16::try_from(FAMILY.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(FAMILY.as_bytes());
        let (k_i, mu_i) = (self.inner[0].k, self.inner[0].mu);
        for v in [self.outer.k, self.outer.mu, k_i, mu_i] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.lambda_decl.to_bytes());
        for t in std::iter::once(&self.outer).chain(&self.inner) {
            for v in t.keep.iter().chain(&t.alt) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

/// Parses a `sa-nested-alias-v1` payload and checks it against the sos-sa spec.
///
/// # Errors
/// A malformed payload, a spec that is not sos-sa, or an inconsistent table.
pub fn parse(payload: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let d = sa(spec)?;
    let u32_at = |at: usize| -> Result<u32, String> {
        let s = payload
            .get(at..at + 4)
            .ok_or(format!("{FAMILY}: truncated"))?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let (k_o, mu_o, k_i, mu_i) = (u32_at(0)?, u32_at(4)?, u32_at(8)?, u32_at(12)?);
    if k_o > MAX_K || k_i > MAX_K {
        return Err(format!("{FAMILY}: k_o = {k_o} or k_i = {k_i} out of range"));
    }
    let (lambda_decl, used) =
        Exact::from_bytes(payload.get(16..).ok_or(format!("{FAMILY}: truncated"))?)?;
    let mut at = 16 + used;
    let tables = d.outer_items();
    let want = at + 8 * (1usize << k_o) + tables * 8 * (1usize << k_i);
    if payload.len() != want {
        return Err(format!(
            "{FAMILY}: payload length {} does not match the widths and the spec ({want})",
            payload.len()
        ));
    }
    let mut table = |k: u32, mu: u32| -> Result<Table, String> {
        let b = 1usize << k;
        let read = |off: usize| -> Result<Vec<u32>, String> {
            (0..b).map(|i| u32_at(off + 4 * i)).collect()
        };
        let t = Table {
            k,
            mu,
            keep: read(at)?,
            alt: read(at + 4 * b)?,
        };
        at += 8 * b;
        Ok(t)
    };
    let outer = table(k_o, mu_o)?;
    let inner = (0..tables)
        .map(|_| table(k_i, mu_i))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Box::new(SaNestedMap::new(lambda_decl, outer, inner, d)?))
}

/// Float64 largest-remainder counts toward `weights`, summing to `2^bits` (a builder helper; the
/// harness trusts only the exact rounding error).
#[must_use]
pub fn lr_counts(weights: &[f64], bits: u32) -> Vec<u64> {
    let lanes = 1u64 << bits;
    let total: f64 = weights.iter().sum();
    let ideal: Vec<f64> = weights.iter().map(|w| w / total * lanes as f64).collect();
    let mut counts: Vec<u64> = ideal.iter().map(|x| x.floor() as u64).collect();
    let short = lanes.saturating_sub(counts.iter().sum());
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by(|&a, &b| (ideal[b] - ideal[b].floor()).total_cmp(&(ideal[a] - ideal[a].floor())));
    for &i in order
        .iter()
        .cycle()
        .take(usize::try_from(short).unwrap_or(0))
    {
        counts[i] += 1;
    }
    counts
}

/// Builds a lane map for `spec` (spec/SPEC-SA.md section 5, "Constructor"): outer weights
/// `2 |e_r|` per one-body eigenvector (both spins) and `S_rc^2 / 2` per square; inner weights
/// `(1, 1)` per one-body item and `(|w_rc0|, ..., |w_rc,B-1|, |wB_rc|)` per square; float64
/// largest-remainder counts, Walker tables, and `lambda_decl` = the spec's `Lambda`, exactly.
///
/// # Errors
/// More items than buckets, or `u > 63`.
pub fn build(
    spec: &SaSpec,
    (k_o, mu_o): (u32, u32),
    (k_i, mu_i): (u32, u32),
) -> Result<SaNestedMap, String> {
    let mut outer_w: Vec<f64> = spec.e.iter().map(|x| 2.0 * x.abs()).collect();
    for q in 0..spec.r * spec.c {
        let s = spec.wb[q].abs()
            + spec.w[q * spec.b..(q + 1) * spec.b]
                .iter()
                .map(|x| x.abs())
                .sum::<f64>();
        outer_w.push(s * s / 2.0);
    }
    if outer_w.len() > 1 << k_o {
        return Err(format!(
            "{FAMILY}: {} outer items do not fit 2^{k_o}",
            outer_w.len()
        ));
    }
    if spec.b + 1 > 1 << k_i {
        return Err(format!(
            "{FAMILY}: {} inner items do not fit 2^{k_i}",
            spec.b + 1
        ));
    }
    let outer = Table::from_counts(k_o, mu_o, &lr_counts(&outer_w, k_o + mu_o))?;
    let mut inner = Vec::with_capacity(outer_w.len());
    for _ in 0..spec.n {
        inner.push(Table::from_counts(
            k_i,
            mu_i,
            &lr_counts(&[1.0, 1.0], k_i + mu_i),
        )?);
    }
    for q in 0..spec.r * spec.c {
        let mut wts: Vec<f64> = spec.w[q * spec.b..(q + 1) * spec.b]
            .iter()
            .map(|x| x.abs())
            .collect();
        wts.push(spec.wb[q].abs());
        inner.push(Table::from_counts(k_i, mu_i, &lr_counts(&wts, k_i + mu_i))?);
    }
    SaNestedMap::new(spec.lambda.clone(), outer, inner, spec)
}
