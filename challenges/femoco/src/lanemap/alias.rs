//! Lane-map family `alias-v1`: a flat alias table over a spec's `flat_terms` (spec/DESIGN.md
//! section 6).
//!
//! `k` index bits and `mu` keep bits, `u = k + mu`. With `i = s mod 2^k` and `r = s >> k`, lane
//! `s` maps to term `i` if `r < keep[i]` and to `alt[i]` otherwise. Payload: `u32 k, u32 mu`,
//! `lambda_decl` (`Exact::to_bytes`, dyadic, positive), then `keep[2^k]` and `alt[2^k]` as
//! little-endian `u32`.
use super::LaneMap;
use crate::spec::{EncodingSpec, Exact, SystemOp};
use num_bigint::{BigInt, BigUint};
use std::sync::OnceLock;

pub const FAMILY: &str = "alias-v1";
/// Tables above `2^MAX_K` buckets are refused (the file would be gigabytes).
pub const MAX_K: u32 = 26;
/// Largest accepted `e` in `lambda_decl = m / 2^e`.
pub const MAX_LAMBDA_EXPONENT: u32 = 512;

pub struct AliasMap {
    pub k: u32,
    pub mu: u32,
    pub lambda_decl: Exact,
    pub keep: Vec<u32>,
    pub alt: Vec<u32>,
    terms: OnceLock<Vec<SystemOp>>,
}

impl AliasMap {
    /// Builds and checks a table (the same checks `parse` applies, minus the spec's term count).
    ///
    /// # Errors
    /// Wrong table sizes, `k + mu > 63`, `keep >= 2^mu`, or a non-dyadic or non-positive
    /// `lambda_decl`.
    pub fn new(
        k: u32,
        mu: u32,
        lambda_decl: Exact,
        keep: Vec<u32>,
        alt: Vec<u32>,
    ) -> Result<Self, String> {
        if k > MAX_K || k + mu > 63 || mu > 31 {
            return Err(format!("alias-v1: k={k}, mu={mu} out of range"));
        }
        let buckets = 1usize << k;
        if keep.len() != buckets || alt.len() != buckets {
            return Err(format!(
                "alias-v1: tables must have 2^k = {buckets} entries"
            ));
        }
        if let Some(i) = keep.iter().position(|&v| u64::from(v) >= 1u64 << mu) {
            return Err(format!(
                "alias-v1: keep[{i}] = {} is not below 2^mu",
                keep[i]
            ));
        }
        // A bounded exponent keeps the exact rounding error cheap: an unbounded one lets a
        // lane map force megabit-wide arithmetic on every term (denial of service).
        if lambda_decl
            .dyadic_exponent()
            .is_some_and(|e| e > MAX_LAMBDA_EXPONENT)
        {
            return Err(format!(
                "alias-v1: lambda_decl exponent above {MAX_LAMBDA_EXPONENT}"
            ));
        }
        if lambda_decl.dyadic_exponent().is_none() || lambda_decl <= Exact::zero() {
            return Err(format!(
                "alias-v1: lambda_decl {lambda_decl} must be positive and dyadic"
            ));
        }
        Ok(Self {
            k,
            mu,
            lambda_decl,
            keep,
            alt,
            terms: OnceLock::new(),
        })
    }

    /// An exact alias table realizing `counts` (which must sum to `2^(k + mu)`, with at most
    /// `2^k` terms): bucket `t` keeps its own term for part of its lanes and sends the rest to a
    /// term with surplus (Walker/Vose, in integers).
    ///
    /// # Errors
    /// Counts that do not sum to `2^u`, or more terms than buckets.
    pub fn from_counts(
        k: u32,
        mu: u32,
        lambda_decl: Exact,
        counts: &[u64],
    ) -> Result<Self, String> {
        let (buckets, cap) = (1usize << k, 1u64 << mu);
        if counts.len() > buckets || counts.iter().sum::<u64>() != cap << k {
            return Err("alias-v1: counts must sum to 2^u over at most 2^k terms".into());
        }
        let mut left: Vec<u64> = (0..buckets)
            .map(|t| counts.get(t).copied().unwrap_or(0))
            .collect();
        let (mut keep, mut alt) = (vec![0u32; buckets], (0..buckets as u32).collect::<Vec<_>>());
        let mut small: Vec<usize> = (0..buckets).filter(|&t| left[t] < cap).collect();
        let mut large: Vec<usize> = (0..buckets).filter(|&t| left[t] > cap).collect();
        while let Some(s) = small.pop() {
            let Some(&l) = large.last() else { break };
            keep[s] = u32::try_from(left[s]).map_err(|e| e.to_string())?;
            alt[s] = u32::try_from(l).map_err(|e| e.to_string())?;
            left[l] -= cap - left[s];
            if left[l] <= cap {
                large.pop();
                if left[l] < cap {
                    small.push(l);
                }
            }
        }
        // Buckets left exactly full send every lane to themselves via alt (keep stays < 2^mu).
        Self::new(k, mu, lambda_decl, keep, alt)
    }

    /// Lanes per term: `n_t = keep[t] + sum over buckets i with alt[i] = t of (2^mu - keep[i])`.
    #[must_use]
    pub fn counts(&self, terms: usize) -> Vec<u64> {
        let cap = 1u64 << self.mu;
        let mut n = vec![0u64; terms.max(1 << self.k)];
        for (i, (&kp, &a)) in self.keep.iter().zip(&self.alt).enumerate() {
            n[i] += u64::from(kp);
            n[a as usize] += cap - u64::from(kp);
        }
        n.truncate(terms);
        n
    }

    fn term_of(&self, s: u64) -> usize {
        let i = (s & ((1u64 << self.k) - 1)) as usize;
        let r = s >> self.k;
        if r < u64::from(self.keep[i]) {
            i
        } else {
            self.alt[i] as usize
        }
    }
}

/// Largest-remainder integer counts for weights `c_t >= 0`: `n_t ~ 2^u c_t / sum(c)`, summing
/// to exactly `2^u`. A helper for building tables; the harness never trusts it.
///
/// # Errors
/// A negative or all-zero weight vector.
pub fn largest_remainder_counts(c: &[Exact], u: u32) -> Result<Vec<u64>, String> {
    let total = c.iter().fold(Exact::zero(), |a, x| a.add(x));
    if c.iter().any(Exact::is_negative) || total.is_zero() {
        return Err("weights must be non-negative and not all zero".into());
    }
    let scale = Exact::new(BigInt::from(1u64 << u), BigUint::from(1u8));
    let mut parts: Vec<(u64, Exact, usize)> = Vec::with_capacity(c.len());
    for (t, x) in c.iter().enumerate() {
        let q = x.mul(&scale).mul(&Exact::new(
            total.den.clone().into(),
            total.num.magnitude().clone(),
        ));
        let fl = &q.num / BigInt::from(q.den.clone());
        let fl_u = u64::try_from(fl.clone()).map_err(|e| e.to_string())?;
        parts.push((fl_u, q.sub(&Exact::new(fl, BigUint::from(1u8))), t));
    }
    let mut n: Vec<u64> = parts.iter().map(|p| p.0).collect();
    let short = (1u64 << u) - n.iter().sum::<u64>();
    parts.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    for p in parts
        .iter()
        .take(usize::try_from(short).map_err(|e| e.to_string())?)
    {
        n[p.2] += 1;
    }
    Ok(n)
}

impl LaneMap for AliasMap {
    fn family(&self) -> &str {
        FAMILY
    }
    fn uniform_bits(&self) -> u32 {
        self.k + self.mu
    }
    fn lambda_decl(&self) -> Exact {
        self.lambda_decl.clone()
    }
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp {
        let terms = self.terms.get_or_init(|| {
            spec.flat_terms()
                .unwrap_or_default()
                .into_iter()
                .map(|(_, op)| op)
                .collect()
        });
        terms[self.term_of(s)].clone()
    }
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String> {
        let terms = spec
            .flat_terms()
            .ok_or("alias-v1: spec has no flat term list")?;
        check_against(self, terms.len())?;
        let two_u = Exact::new(BigInt::from(1u8), BigUint::from(1u8) << self.uniform_bits());
        let unit = self.lambda_decl.mul(&two_u);
        let mut err = Exact::zero();
        for ((c, _), n) in terms.iter().zip(self.counts(terms.len())) {
            if c.is_negative() {
                return Err("alias-v1: spec coefficient is negative (signs belong in M_t)".into());
            }
            let got = unit.mul(&Exact::new(BigInt::from(n), BigUint::from(1u8)));
            err = err.add(&c.sub(&got).abs());
        }
        Ok(err)
    }
    fn alias_bits(&self) -> Option<(u32, u32)> {
        Some((self.k, self.mu))
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = super::MAGIC.to_vec();
        out.extend_from_slice(&u16::try_from(FAMILY.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(FAMILY.as_bytes());
        out.extend_from_slice(&self.k.to_le_bytes());
        out.extend_from_slice(&self.mu.to_le_bytes());
        out.extend_from_slice(&self.lambda_decl.to_bytes());
        for v in self.keep.iter().chain(&self.alt) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// Every alias target is a real term and every padding bucket (`i >= L`) aliases all its lanes.
fn check_against(m: &AliasMap, terms: usize) -> Result<(), String> {
    if let Some(i) = m.alt.iter().position(|&a| a as usize >= terms) {
        return Err(format!(
            "alias-v1: alt[{i}] = {} is not a term (L = {terms})",
            m.alt[i]
        ));
    }
    if let Some(i) = (terms..m.keep.len()).find(|&i| m.keep[i] != 0) {
        return Err(format!(
            "alias-v1: bucket {i} >= L = {terms} must have keep = 0"
        ));
    }
    Ok(())
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, String> {
    let s = b.get(at..at + 4).ok_or("alias-v1: truncated")?;
    Ok(u32::from_le_bytes(
        s.try_into().map_err(|_| "alias-v1: truncated")?,
    ))
}

/// Parses an `alias-v1` payload and checks it against the spec's flat terms.
///
/// # Errors
/// Malformed payload, a spec without flat terms, or an inconsistent table.
pub fn parse(payload: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let (k, mu) = (u32_at(payload, 0)?, u32_at(payload, 4)?);
    if k > MAX_K {
        return Err(format!("alias-v1: k = {k} above {MAX_K}"));
    }
    let (lambda_decl, used) = Exact::from_bytes(&payload[8..])?;
    let at = 8 + used;
    let buckets = 1usize << k;
    if payload.len() != at + 8 * buckets {
        return Err("alias-v1: payload length does not match 2^k".into());
    }
    let table = |off: usize| {
        (0..buckets)
            .map(|i| u32_at(payload, off + 4 * i))
            .collect::<Result<Vec<_>, _>>()
    };
    let m = AliasMap::new(k, mu, lambda_decl, table(at)?, table(at + 4 * buckets)?)?;
    let terms = spec
        .flat_terms()
        .ok_or("alias-v1: spec has no flat term list")?;
    check_against(&m, terms.len())?;
    Ok(Box::new(m))
}
