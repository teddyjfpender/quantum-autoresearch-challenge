//! Lane-map family `thc-pair-alias-v1` (the THC encoding; no THC spec ships here).
//!
//! An alias table over the thc spec's *pairs* (one-body eigenvector `k`, or unordered factor pair
//! `mu <= nu`), plus two spin bits and a swap bit. With `k` index bits and `mu` keep bits,
//! `u = k + mu + 3`; lane `s` has `i = s mod 2^k`, `r = (s >> k) mod 2^mu`, `s1 = bit k + mu`,
//! `s2 = bit k + mu + 1`, `b = bit k + mu + 2`, and names pair `i` if `r < keep[i]` else
//! `alt[i]`. For `b = 0` the lane applies `Z(chi_mu, s1) Z(chi_nu, s2)` and for `b = 1` the
//! reversed product `Z(chi_nu, s2) Z(chi_mu, s1)`; one-body lanes apply `Z(w_k, s1)` whatever
//! `s2` and `b`; `mu = nu`, `s1 = s2` lanes apply `sign(zeta_mumu) I` (a known energy shift). Payload:
//! exactly the `alias-v1` payload (`u32 k`, `u32 mu`, `lambda_decl`, `keep[2^k]`, `alt[2^k]`).
//!
//! **The swap bit.** For `mu != nu` the two factors do not commute (THC factors are not
//! orthogonal), so `Z(chi_mu) Z(chi_nu)` is unitary but not Hermitian, and a SELECT that applied
//! it on a fixed lane would not be a reflection. Following Lee et al. 2021 Sec. III.C, Eqs.
//! (36)-(42) and Fig. 6, the controlled block encoding also flips `b` (an `X` on the qubit that
//! controls the `mu`/`nu` swap): a control-1 lane `s` must end with the uniform register holding
//! `pi(s) = s XOR 2^(k + mu + 2)` (`LaneMap::uniform_after`). Since the op of lane `pi(s)` is the
//! adjoint of the op of lane `s` by construction, `B = sum_s |pi(s)><s| (x) M_s` is Hermitian and
//! self-inverse, and `<+|B|+> = sum_s M_s / 2^u` is unchanged.
//!
//! Counts are exact: every `(s1, s2, b)` combination of pair `P` gets `n_P` lanes, where `n_P` is
//! the alias count. A one-body term (one spin) has `4 n_P` lanes, a `mu < nu` term (spins and
//! order) `n_P`, and a `mu = nu` term (`s1 != s2`, one order) `2 n_P`.
use super::alias::AliasMap;
use super::LaneMap;
use crate::spec::thc::{Pair, ThcSpec};
use crate::spec::{EncodingSpec, Exact, Monomial, SystemOp};
use num_bigint::BigInt;
use num_traits::Signed;

pub const FAMILY: &str = "thc-pair-alias-v1";

pub struct ThcPairMap {
    pub table: AliasMap,
}

fn thc(spec: &dyn EncodingSpec) -> Result<&ThcSpec, String> {
    spec.as_any()
        .downcast_ref::<ThcSpec>()
        .ok_or_else(|| format!("{FAMILY}: spec {} is not a thc spec", spec.id()))
}

/// What one lane selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane {
    pub pair: u64,
    pub s1: u8,
    pub s2: u8,
    /// The swap bit `b`.
    pub swap: bool,
}

impl ThcPairMap {
    /// Checks the table against the spec: `k + mu + 3 <= 63`, every alias target is a pair, and
    /// every bucket past the last pair sends all its lanes elsewhere.
    ///
    /// # Errors
    /// A table inconsistent with the spec.
    pub fn new(table: AliasMap, spec: &ThcSpec) -> Result<Self, String> {
        let pairs = spec.pair_count();
        if table.k + table.mu + 3 > 63 {
            return Err(format!(
                "{FAMILY}: k + mu + 3 = {} above 63",
                table.k + table.mu + 3
            ));
        }
        if let Some(i) = table.alt.iter().position(|&a| u64::from(a) >= pairs) {
            return Err(format!(
                "{FAMILY}: alt[{i}] = {} is not a pair ({pairs} pairs)",
                table.alt[i]
            ));
        }
        if let Some(i) = (0..table.keep.len()).find(|&i| i as u64 >= pairs && table.keep[i] != 0) {
            return Err(format!(
                "{FAMILY}: bucket {i} is not a pair and must have keep = 0"
            ));
        }
        Ok(Self { table })
    }

    /// Lanes per pair (each `(s1, s2, b)` combination), `n_P`.
    #[must_use]
    pub fn counts(&self, pairs: u64) -> Vec<u64> {
        self.table
            .counts(usize::try_from(pairs).unwrap_or(usize::MAX))
    }

    /// Bit position of the swap bit `b` in the uniform register: `k + mu + 2`.
    #[must_use]
    pub fn swap_bit(&self) -> u32 {
        self.table.k + self.table.mu + 2
    }

    /// Lane `s` -> (pair, s1, s2, b).
    #[must_use]
    pub fn decode(&self, s: u64) -> Lane {
        let t = &self.table;
        let i = (s & ((1u64 << t.k) - 1)) as usize;
        let r = (s >> t.k) & ((1u64 << t.mu) - 1);
        let pair = if r < u64::from(t.keep[i]) {
            i as u64
        } else {
            u64::from(t.alt[i])
        };
        let hi = s >> (t.k + t.mu);
        Lane {
            pair,
            s1: (hi & 1) as u8,
            s2: (hi >> 1 & 1) as u8,
            swap: hi >> 2 & 1 == 1,
        }
    }
}

/// Target lanes-per-combination weights (8 combinations `(s1, s2, b)` per pair): one-body
/// `|t_k| / 8`, `mu < nu` `|zeta| / 8`, `mu = nu` `|zeta| / 16`. Summed over the combinations
/// this gives `lambda_lanes = lambda_T + (1/2) sum_{mu nu} |zeta|` (Lee et al. 2021 Eq. (20)).
#[must_use]
pub fn combination_weight(spec: &ThcSpec, p: Pair) -> f64 {
    match p {
        Pair::OneBody { k } => spec.t[k].abs() / 8.0,
        Pair::TwoBody { mu, nu } if mu == nu => spec.zeta(mu, nu).abs() / 16.0,
        Pair::TwoBody { mu, nu } => spec.zeta(mu, nu).abs() / 8.0,
    }
}

/// Lanes each of the pair's terms gets when the pair has `n` lanes per combination.
#[must_use]
pub fn lanes_per_term(p: Pair, n: u64) -> u64 {
    match p {
        Pair::OneBody { .. } => 4 * n,
        Pair::TwoBody { mu, nu } if mu == nu => 2 * n,
        Pair::TwoBody { .. } => n,
    }
}

/// Builds a table for `spec` with `k` index and `mu` keep bits: counts by largest remainder in
/// float64 (a helper for baselines; the harness trusts only the exact `rounding_error`), and
/// `lambda_decl = spec.lambda_lanes` exactly (every combination, identity lanes included).
///
/// # Errors
/// More pairs than `2^k`, or an out-of-range `k`/`mu`.
pub fn build(spec: &ThcSpec, k: u32, mu: u32) -> Result<ThcPairMap, String> {
    let pairs = spec.pair_count();
    if pairs > 1u64 << k {
        return Err(format!("{FAMILY}: {pairs} pairs do not fit 2^{k} buckets"));
    }
    let ws: Vec<f64> = (0..pairs)
        .filter_map(|i| spec.pair(i))
        .map(|p| combination_weight(spec, p))
        .collect();
    let lanes = 1u64 << (k + mu);
    let total: f64 = ws.iter().sum();
    let ideal: Vec<f64> = ws.iter().map(|w| w / total * lanes as f64).collect();
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
    let table = AliasMap::from_counts(k, mu, spec.lambda_lanes.clone(), &counts)?;
    ThcPairMap::new(table, spec)
}

/// A dyadic rational `m / 2^e` without gcd reduction (fast exact sums of many dyadics).
#[derive(Clone)]
struct Dy {
    m: BigInt,
    e: u32,
}

impl Dy {
    fn from_exact(x: &Exact) -> Result<Self, String> {
        let e = x.dyadic_exponent().ok_or("thc-pair: value is not dyadic")?;
        Ok(Self {
            m: x.num.clone(),
            e,
        })
    }
    fn align(&self, e: u32) -> BigInt {
        &self.m << (e - self.e)
    }
    fn add_abs_diff(&mut self, a: &Self, b: &Self, times: u64) {
        let e = self.e.max(a.e).max(b.e);
        let d = (a.align(e) - b.align(e)).abs() * times;
        self.m = self.align(e) + d;
        self.e = e;
    }
}

impl LaneMap for ThcPairMap {
    fn family(&self) -> &str {
        FAMILY
    }
    fn uniform_bits(&self) -> u32 {
        self.table.k + self.table.mu + 3
    }
    fn lambda_decl(&self) -> Exact {
        self.table.lambda_decl.clone()
    }
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp {
        let identity = SystemOp::Monomial(Monomial {
            phase: 0,
            majoranas: Vec::new(),
        });
        let Ok(t) = thc(spec) else { return identity };
        let l = self.decode(s);
        t.pair(l.pair)
            .map_or(identity, |p| t.lane_op(p, l.s1, l.s2, l.swap))
    }
    fn uniform_after(&self, s: u64) -> u64 {
        s ^ (1u64 << self.swap_bit())
    }
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String> {
        let t = thc(spec)?;
        let pairs = t.pair_count();
        let unit = self
            .table
            .lambda_decl
            .mul(&Exact::dyadic(1.into(), self.uniform_bits()));
        let unit = Dy::from_exact(&unit)?;
        let mut err = Dy {
            m: BigInt::from(0),
            e: 0,
        };
        for (i, n) in self.counts(pairs).into_iter().enumerate() {
            let p = t
                .pair(i as u64)
                .ok_or("thc-pair: pair index out of range")?;
            let c = Dy::from_exact(&t.term_magnitude(p))?;
            let got = Dy {
                m: &unit.m * lanes_per_term(p, n),
                e: unit.e,
            };
            err.add_abs_diff(&c, &got, ThcSpec::terms_in(p));
        }
        Ok(Exact::dyadic(err.m, err.e))
    }
    fn alias_bits(&self) -> Option<(u32, u32)> {
        Some((self.table.k, self.table.mu))
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = super::MAGIC.to_vec();
        out.extend_from_slice(&u16::try_from(FAMILY.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(FAMILY.as_bytes());
        // The alias-v1 payload: everything after its own family header.
        let inner = self.table.to_bytes();
        out.extend_from_slice(&inner[super::MAGIC.len() + 2 + super::alias::FAMILY.len()..]);
        out
    }
}

/// Parses a `thc-pair-alias-v1` payload and checks it against the thc spec.
///
/// # Errors
/// A malformed payload, a spec that is not thc, or an inconsistent table.
pub fn parse(payload: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let t = thc(spec)?;
    let u32_at = |at: usize| -> Result<u32, String> {
        let s = payload.get(at..at + 4).ok_or("thc-pair: truncated")?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let (k, mu) = (u32_at(0)?, u32_at(4)?);
    if k > super::alias::MAX_K {
        return Err(format!("{FAMILY}: k = {k} out of range"));
    }
    let (lambda_decl, used) = Exact::from_bytes(payload.get(8..).ok_or("thc-pair: truncated")?)?;
    let at = 8 + used;
    let buckets = 1usize << k;
    if payload.len() != at + 8 * buckets {
        return Err(format!("{FAMILY}: payload length does not match 2^k"));
    }
    let table = |off: usize| {
        (0..buckets)
            .map(|i| u32_at(off + 4 * i))
            .collect::<Result<Vec<_>, _>>()
    };
    let a = AliasMap::new(k, mu, lambda_decl, table(at)?, table(at + 4 * buckets)?)?;
    Ok(Box::new(ThcPairMap::new(a, t)?))
}
