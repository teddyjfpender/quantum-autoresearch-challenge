//! Lane-map family `df-pair-alias-v1` (the DF encoding; no DF spec ships here).
//!
//! An alias table over the df spec's *pairs* (one-body eigenvector `k`, or leaf `l` with
//! eigenvectors `(k1, k2)`), plus two spin bits. With `k` index bits and `mu` keep bits,
//! `u = k + mu + 2`; lane `s` has `i = s mod 2^k`, `r = (s >> k) mod 2^mu`,
//! `s1 = bit k + mu`, `s2 = bit k + mu + 1`, and names pair `i` if `r < keep[i]` else `alt[i]`.
//! The lane applies that pair's term at spins `(s1, s2)` (one-body terms ignore `s2`); a
//! two-body `k1 = k2`, `s1 = s2` lane applies the identity, which only shifts the encoded
//! operator by a known multiple of `I`. Payload: exactly the `alias-v1` payload (`u32 k`,
//! `u32 mu`, `lambda_decl`, `keep[2^k]`, `alt[2^k]`).
//!
//! Counts are exact: every spin combination of pair `P` gets `n_P` lanes, where `n_P` is the
//! alias count, so a one-body term has `2 n_P` lanes and a two-body term `n_P`.
use super::alias::AliasMap;
use super::LaneMap;
use crate::spec::df::{DfForm, DfSpec, Pair};
use crate::spec::{EncodingSpec, Exact, Monomial, SystemOp};
use num_bigint::BigInt;
use num_traits::Signed;

pub const FAMILY: &str = "df-pair-alias-v1";

pub struct DfPairMap {
    pub table: AliasMap,
}

fn df(spec: &dyn EncodingSpec) -> Result<&DfSpec, String> {
    spec.as_any()
        .downcast_ref::<DfSpec>()
        .ok_or_else(|| format!("{FAMILY}: spec {} is not a df spec", spec.id()))
}

impl DfPairMap {
    /// Checks the table against the spec: a flat-form spec, `k + mu + 2 <= 63`, every alias
    /// target is a pair, and every bucket past the last pair sends all its lanes elsewhere.
    ///
    /// # Errors
    /// A table inconsistent with the spec.
    pub fn new(table: AliasMap, spec: &DfSpec) -> Result<Self, String> {
        if spec.form != DfForm::Flat {
            return Err(format!(
                "{FAMILY}: spec {} is a nested-form df spec; use df-nested-alias-v1",
                spec.id
            ));
        }
        let pairs = spec.pair_count();
        if table.k + table.mu + 2 > 63 {
            return Err(format!(
                "{FAMILY}: k + mu + 2 = {} above 63",
                table.k + table.mu + 2
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

    /// Lanes per pair (each spin combination), `n_P`.
    #[must_use]
    pub fn counts(&self, pairs: u64) -> Vec<u64> {
        self.table
            .counts(usize::try_from(pairs).unwrap_or(usize::MAX))
    }

    /// Lane `s` -> (pair, s1, s2).
    #[must_use]
    pub fn decode(&self, s: u64) -> (u64, u8, u8) {
        let t = &self.table;
        let i = (s & ((1u64 << t.k) - 1)) as usize;
        let r = (s >> t.k) & ((1u64 << t.mu) - 1);
        let pair = if r < u64::from(t.keep[i]) {
            i as u64
        } else {
            u64::from(t.alt[i])
        };
        let spins = s >> (t.k + t.mu);
        (pair, (spins & 1) as u8, (spins >> 1 & 1) as u8)
    }
}

/// Target lanes-per-combination weights: one-body `|t_k| / 4` (two `s2` values share a term of
/// weight `|t_k| / 2`), two-body `|e_{l k1} e_{l k2}| / 8`; summed over the 4 combinations this
/// gives `lambda_lanes = lambda_T + sum_l S_l^2 / 2`.
fn pair_weight_f64(spec: &DfSpec, p: Pair) -> f64 {
    match p {
        Pair::OneBody { k } => spec.t[k].abs(),
        Pair::TwoBody { l, k1, k2 } => (spec.leaves[l].e[k1] * spec.leaves[l].e[k2]).abs() / 2.0,
    }
}

/// Builds a table for `spec` with `k` index and `mu` keep bits: counts by largest remainder in
/// float64 (a helper for the baseline; the harness trusts only the exact `rounding_error`), and
/// `lambda_decl = sum over pairs of 4 * (per-combination weight)`, exactly.
///
/// # Errors
/// More pairs than `2^k`, or an out-of-range `k`/`mu`.
pub fn build(spec: &DfSpec, k: u32, mu: u32) -> Result<DfPairMap, String> {
    let pairs = spec.pair_count();
    if pairs > 1u64 << k {
        return Err(format!("{FAMILY}: {pairs} pairs do not fit 2^{k} buckets"));
    }
    let ws: Vec<f64> = (0..pairs)
        .filter_map(|i| spec.pair(i))
        .map(|p| pair_weight_f64(spec, p))
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
    let lambda = (0..pairs)
        .filter_map(|i| spec.pair(i))
        .fold(Exact::zero(), |a, p| {
            let per = spec.term_magnitude(p);
            let combos = if matches!(p, Pair::OneBody { .. }) {
                2
            } else {
                4
            };
            a.add(&per.mul(&Exact::from_int(combos)))
        });
    let table = AliasMap::from_counts(k, mu, lambda, &counts)?;
    DfPairMap::new(table, spec)
}

/// A dyadic rational `m / 2^e` without gcd reduction (fast exact sums of many dyadics).
#[derive(Clone)]
struct Dy {
    m: BigInt,
    e: u32,
}

impl Dy {
    fn from_exact(x: &Exact) -> Result<Self, String> {
        let e = x.dyadic_exponent().ok_or("df-pair: value is not dyadic")?;
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

impl LaneMap for DfPairMap {
    fn family(&self) -> &str {
        FAMILY
    }
    fn uniform_bits(&self) -> u32 {
        self.table.k + self.table.mu + 2
    }
    fn lambda_decl(&self) -> Exact {
        self.table.lambda_decl.clone()
    }
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp {
        let identity = SystemOp::Monomial(Monomial {
            phase: 0,
            majoranas: Vec::new(),
        });
        let Ok(d) = df(spec) else { return identity };
        let (pair, s1, s2) = self.decode(s);
        d.pair(pair)
            .and_then(|p| d.term_op(p, s1, s2))
            .unwrap_or(identity)
    }
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String> {
        let d = df(spec)?;
        let pairs = d.pair_count();
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
            let p = d.pair(i as u64).ok_or("df-pair: pair index out of range")?;
            let c = Dy::from_exact(&d.term_magnitude(p))?;
            let lanes_per_term = if matches!(p, Pair::OneBody { .. }) {
                2 * n
            } else {
                n
            };
            let got = Dy {
                m: &unit.m * lanes_per_term,
                e: unit.e,
            };
            err.add_abs_diff(&c, &got, DfSpec::terms_in(p));
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

/// Parses a `df-pair-alias-v1` payload and checks it against the df spec.
///
/// # Errors
/// A malformed payload, a spec that is not df, or an inconsistent table.
pub fn parse(payload: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let d = df(spec)?;
    let u32_at = |at: usize| -> Result<u32, String> {
        let s = payload.get(at..at + 4).ok_or("df-pair: truncated")?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let (k, mu) = (u32_at(0)?, u32_at(4)?);
    if k > super::alias::MAX_K {
        return Err(format!("{FAMILY}: k = {k} out of range"));
    }
    let (lambda_decl, used) = Exact::from_bytes(payload.get(8..).ok_or("df-pair: truncated")?)?;
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
    let t = AliasMap::new(k, mu, lambda_decl, table(at)?, table(at + 4 * buckets)?)?;
    Ok(Box::new(DfPairMap::new(t, d)?))
}
