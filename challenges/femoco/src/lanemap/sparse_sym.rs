//! Lane-map family `sparse-sym-alias-v1`. See spec/DESIGN.md section 6;
//! the sparse encoding; no sparse spec ships here.
//!
//! An alias table over the sparse spec's entries times the five expansion bits of
//! [`crate::spec::sparse::rule`]. `u = k + mu + 5`. Lane `s` splits as `i = s mod 2^k`,
//! `r = (s >> k) mod 2^mu`, `x = s >> (k + mu)`; it selects entry `e = i` if `r < keep[i]`, else
//! `alt[i]`, and must apply `expand(e, x)`. Every entry is therefore selected on the same number
//! `n_e` of lanes for each of its 32 expansion values, and the encoded operator is
//! `lambda_decl / 2^(k+mu) * sum_e n_e (1/32) sum_x M_{e,x}`.
pub mod build;

use super::{LaneMap, MAGIC};
use crate::spec::sparse::dyadic::{self, Dyadic};
use crate::spec::sparse::rule::EXPANSION_BITS;
use crate::spec::sparse::SparseSpec;
use crate::spec::{EncodingSpec, Exact, SystemOp};
use num_bigint::{BigInt, BigUint, Sign};

pub const FAMILY: &str = "sparse-sym-alias-v1";
/// Largest accepted `k` (tables of `2^k` u32 pairs) and `k + mu + 5`.
const MAX_K: u32 = 26;
const MAX_U: u32 = 62;
/// Largest accepted `shift` of `lambda_decl = mag / 2^shift` (`mag` is at most 512 bits).
const MAX_SHIFT: u32 = 512;

pub struct SparseSymAlias {
    pub k: u32,
    pub mu: u32,
    /// Declared normalization, a positive dyadic.
    pub lambda_decl: Dyadic,
    pub keep: Vec<u32>,
    pub alt: Vec<u32>,
}

fn sparse(spec: &dyn EncodingSpec) -> Result<&SparseSpec, String> {
    spec.as_any()
        .downcast_ref::<SparseSpec>()
        .ok_or_else(|| format!("{FAMILY} needs a sparse spec, got {}", spec.encoding()))
}

impl SparseSymAlias {
    /// Checks the declared data against the spec: sizes, `keep < 2^mu`, `alt` in range,
    /// buckets past the last entry fully aliased, positive `lambda_decl`.
    ///
    /// # Errors
    /// The first violated rule.
    pub fn validate(&self, spec: &SparseSpec) -> Result<(), String> {
        let l = spec.entries().len();
        if self.k > MAX_K || self.k + self.mu + EXPANSION_BITS > MAX_U || self.mu > 31 {
            return Err(format!(
                "{FAMILY}: k = {}, mu = {} out of range",
                self.k, self.mu
            ));
        }
        let buckets = 1usize << self.k;
        if self.keep.len() != buckets || self.alt.len() != buckets {
            return Err(format!(
                "{FAMILY}: tables must have 2^k = {buckets} entries"
            ));
        }
        if self.lambda_decl.num.sign() != Sign::Plus {
            return Err(format!("{FAMILY}: lambda_decl must be positive"));
        }
        for i in 0..buckets {
            if self.keep[i] >= 1u32 << self.mu {
                return Err(format!("{FAMILY}: keep[{i}] = {} >= 2^mu", self.keep[i]));
            }
            if self.alt[i] as usize >= l {
                return Err(format!(
                    "{FAMILY}: alt[{i}] = {} is not an entry (L = {l})",
                    self.alt[i]
                ));
            }
            if i >= l && self.keep[i] != 0 {
                return Err(format!(
                    "{FAMILY}: bucket {i} >= L = {l} must have keep = 0"
                ));
            }
        }
        Ok(())
    }

    /// `u = k + mu + 5`.
    #[must_use]
    pub fn u(&self) -> u32 {
        self.k + self.mu + EXPANSION_BITS
    }

    /// The entry lane `s` selects, and its expansion value.
    #[must_use]
    pub fn lane(&self, s: u64) -> (usize, u8) {
        let i = usize::try_from(s & ((1u64 << self.k) - 1)).unwrap_or(0);
        let r = (s >> self.k) & ((1u64 << self.mu) - 1);
        let x = u8::try_from((s >> (self.k + self.mu)) & 31).unwrap_or(0);
        let e = if r < u64::from(self.keep[i]) {
            i
        } else {
            self.alt[i] as usize
        };
        (e, x)
    }

    /// Exact counts `n_e`: lanes per expansion value that select entry `e`. They sum to
    /// `2^(k+mu)`.
    #[must_use]
    pub fn counts(&self, entries: usize) -> Vec<u64> {
        let mut n = vec![0u64; entries];
        let full = 1u64 << self.mu;
        for (i, (&keep, &alt)) in self.keep.iter().zip(&self.alt).enumerate() {
            if i < entries {
                n[i] += u64::from(keep);
            }
            n[alt as usize] += full - u64::from(keep);
        }
        n
    }

    /// Exact `sum_e |w_e - lambda_decl * n_e / 2^(k+mu)|`, which equals the term-level
    /// `sum_{e,x} |w_e/32 - lambda_decl * n_e / 2^u|` of DESIGN.md section 6. Identical
    /// monomials from different terms are not merged, so this bounds the error of the merged
    /// operator too (triangle inequality).
    #[must_use]
    pub fn rounding_error_dyadic(&self, spec: &SparseSpec) -> Dyadic {
        let counts = self.counts(spec.entries().len());
        let diffs: Vec<Dyadic> = counts
            .iter()
            .enumerate()
            .map(|(e, &n)| {
                let enc = self.lambda_decl.scale(&BigInt::from(n), self.k + self.mu);
                spec.weight(e).sub(&enc).abs()
            })
            .collect();
        dyadic::sum(diffs.iter())
    }

    fn payload(&self) -> Vec<u8> {
        let (_, mag) = self.lambda_decl.num.to_bytes_le();
        let mut b = Vec::with_capacity(16 + mag.len() + 8 * self.keep.len());
        b.push(u8::try_from(self.k).unwrap_or(u8::MAX));
        b.push(u8::try_from(self.mu).unwrap_or(u8::MAX));
        b.extend_from_slice(&self.lambda_decl.shift.to_le_bytes());
        b.extend_from_slice(&u32::try_from(mag.len()).unwrap_or(u32::MAX).to_le_bytes());
        b.extend_from_slice(&mag);
        for v in self.keep.iter().chain(&self.alt) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }
}

impl LaneMap for SparseSymAlias {
    fn family(&self) -> &str {
        FAMILY
    }
    fn uniform_bits(&self) -> u32 {
        self.u()
    }
    fn lambda_decl(&self) -> Exact {
        self.lambda_decl.to_exact()
    }
    /// # Panics
    /// If `spec` is not the sparse spec this map was parsed against (`parse` checks that).
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp {
        let spec = sparse(spec).expect("sparse-sym-alias-v1 used with a non-sparse spec");
        let (e, x) = self.lane(s);
        SystemOp::Monomial(spec.term(e, x))
    }
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String> {
        let spec = sparse(spec)?;
        self.validate(spec)?;
        Ok(self.rounding_error_dyadic(spec).to_exact())
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&u16::try_from(FAMILY.len()).unwrap_or(0).to_le_bytes());
        b.extend_from_slice(FAMILY.as_bytes());
        b.extend_from_slice(&self.payload());
        b
    }
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
    if b.len() < n {
        return Err(format!("{FAMILY}: truncated"));
    }
    let (head, tail) = b.split_at(n);
    *b = tail;
    Ok(head)
}

fn u32s(b: &mut &[u8], n: usize) -> Result<Vec<u32>, String> {
    let raw = take(b, 4 * n)?;
    Ok(raw
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Parses the family payload (after magic and family string) and validates it against `spec`.
///
/// Payload: `u8 k, u8 mu, u32 shift, u32 len, len bytes` (`lambda_decl = mag / 2^shift`, `mag`
/// little-endian unsigned), then `keep[2^k]` and `alt[2^k]` as little-endian u32.
///
/// # Errors
/// Truncated or trailing bytes, or any rule of [`SparseSymAlias::validate`].
pub fn parse(payload: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let sp = sparse(spec)?;
    let mut b = payload;
    let head = take(&mut b, 10)?;
    let (k, mu) = (u32::from(head[0]), u32::from(head[1]));
    let shift = u32::from_le_bytes([head[2], head[3], head[4], head[5]]);
    let len = u32::from_le_bytes([head[6], head[7], head[8], head[9]]) as usize;
    if k > MAX_K || len > 64 {
        return Err(format!(
            "{FAMILY}: k = {k} or lambda length {len} out of range"
        ));
    }
    // `mag` has at most 512 bits, so a larger shift only makes lambda_decl tiny while forcing
    // exact arithmetic on 2^shift-sized integers (a denial of service on the trusted evaluator).
    if shift > MAX_SHIFT {
        return Err(format!("{FAMILY}: lambda shift {shift} above {MAX_SHIFT}"));
    }
    let mag = BigUint::from_bytes_le(take(&mut b, len)?);
    let lambda_decl = Dyadic::new(BigInt::from(mag), shift);
    let keep = u32s(&mut b, 1usize << k)?;
    let alt = u32s(&mut b, 1usize << k)?;
    if !b.is_empty() {
        return Err(format!("{FAMILY}: {} trailing bytes", b.len()));
    }
    let map = SparseSymAlias {
        k,
        mu,
        lambda_decl,
        keep,
        alt,
    };
    map.validate(sp)?;
    Ok(Box::new(map))
}
