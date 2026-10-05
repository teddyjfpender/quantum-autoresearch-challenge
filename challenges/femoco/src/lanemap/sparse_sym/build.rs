//! Constructs a `sparse-sym-alias-v1` lane map from a spec, for baselines and tests.
//!
//! Nothing here is trusted: whatever it builds, `parse`/`validate` and the exact rounding error
//! decide whether it is acceptable. Floating point is used only to choose integer counts.
use super::SparseSymAlias;
use crate::spec::sparse::dyadic::Dyadic;
use crate::spec::sparse::SparseSpec;
use num_bigint::BigInt;

/// Bits after the binary point of the default `lambda_decl` (the spec's lambda, rounded).
pub const LAMBDA_SHIFT: u32 = 40;

/// The spec's exact lambda rounded to the nearest multiple of `2^-LAMBDA_SHIFT`.
#[must_use]
pub fn default_lambda(spec: &SparseSpec) -> Dyadic {
    let l = spec.lambda_dyadic();
    if l.shift <= LAMBDA_SHIFT {
        return l.clone();
    }
    let drop = l.shift - LAMBDA_SHIFT;
    let half = BigInt::from(1) << (drop - 1);
    Dyadic::new((&l.num + half) >> drop, LAMBDA_SHIFT)
}

/// Integer counts `n_e` summing to `2^bits`, proportional to the entry weights: floors of the
/// targets, then the largest remainders get the leftover lanes.
fn counts(spec: &SparseSpec, lambda: &Dyadic, bits: u32) -> Vec<u64> {
    let total = 1u64 << bits;
    let scale = 2f64.powi(i32::try_from(bits).unwrap_or(0)) / lambda.to_f64();
    let targets: Vec<f64> = (0..spec.entries().len())
        .map(|e| spec.weight(e).to_f64() * scale)
        .collect();
    // f64 -> u64 casts saturate; targets are non-negative and at most about 2^bits.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut n: Vec<u64> = targets.iter().map(|t| t.floor() as u64).collect();
    let assigned: u64 = n.iter().sum();
    let mut order: Vec<usize> = (0..n.len()).collect();
    #[allow(clippy::cast_precision_loss)]
    order.sort_by(|&a, &b| (targets[b] - n[b] as f64).total_cmp(&(targets[a] - n[a] as f64)));
    if assigned <= total {
        for &e in order
            .iter()
            .cycle()
            .take(usize::try_from(total - assigned).unwrap_or(0))
        {
            n[e] += 1;
        }
    } else {
        let mut over = assigned - total;
        for &e in order.iter().rev() {
            let d = over.min(n[e]);
            n[e] -= d;
            over -= d;
            if over == 0 {
                break;
            }
        }
    }
    n
}

/// Builds the alias table over `2^k` buckets with `2^mu` lanes each, from integer counts.
///
/// # Errors
/// More entries than buckets, or `k`, `mu` out of the family's range.
pub fn from_spec(spec: &SparseSpec, k: u32, mu: u32) -> Result<SparseSymAlias, String> {
    from_spec_with_lambda(spec, k, mu, default_lambda(spec))
}

/// As [`from_spec`] with a chosen dyadic `lambda_decl`.
///
/// # Errors
/// As [`from_spec`].
pub fn from_spec_with_lambda(
    spec: &SparseSpec,
    k: u32,
    mu: u32,
    lambda_decl: Dyadic,
) -> Result<SparseSymAlias, String> {
    let l = spec.entries().len();
    if k > super::MAX_K || mu > 31 || l > 1usize << k {
        return Err(format!(
            "cannot place {l} entries in 2^{k} buckets of 2^{mu} lanes"
        ));
    }
    let mut cnt = counts(spec, &lambda_decl, k + mu);
    cnt.resize(1usize << k, 0);
    let (keep, alt) = alias(&mut cnt, 1u64 << mu);
    let map = SparseSymAlias {
        k,
        mu,
        lambda_decl,
        keep,
        alt,
    };
    map.validate(spec)?;
    Ok(map)
}

/// Vose's alias method on integer counts that sum to `buckets * cap`. A bucket that keeps all
/// of its lanes is written as `keep = 0, alt = itself`, so every `keep < cap`.
fn alias(cnt: &mut [u64], cap: u64) -> (Vec<u32>, Vec<u32>) {
    let b = cnt.len();
    let id = |i: usize| u32::try_from(i).unwrap_or(u32::MAX);
    let (mut keep, mut alt) = (vec![0u32; b], (0..b).map(id).collect::<Vec<u32>>());
    let mut small: Vec<usize> = (0..b).filter(|&i| cnt[i] < cap).collect();
    let mut large: Vec<usize> = (0..b).filter(|&i| cnt[i] > cap).collect();
    while let Some(s) = small.pop() {
        let Some(&g) = large.last() else { break };
        keep[s] = u32::try_from(cnt[s]).unwrap_or(0);
        alt[s] = id(g);
        cnt[g] -= cap - cnt[s];
        if cnt[g] <= cap {
            large.pop();
            if cnt[g] < cap {
                small.push(g);
            }
        }
    }
    (keep, alt)
}
