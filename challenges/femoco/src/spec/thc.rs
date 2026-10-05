//! The thc spec: tensor hypercontraction with quantized Givens networks (the THC encoding; no THC spec ships here).
//!
//!
//! The operator is `H = identity + sum_{k,s} (-t_k / 2) Z(w_k, s)
//! + (1/8) sum_{(mu,s1) != (nu,s2)} zeta_{mu nu} Z(chi_mu, s1) Z(chi_nu, s2)`, where
//! `Z(u, s) = G_u Z_{mode s} G_u^dagger`, `G_u` is the quantized Givens network that maps spatial
//! orbital 0 to `u` (the df convention, spec/DESIGN.md section 15), `chi_mu` are the THC factors
//! (single normalized vectors, *not* an orthonormal basis) and `zeta` is symmetric. Terms are
//! grouped in *pairs*: pair `k < N` is the one-body eigenvector `k`, then pair
//! `N + nu (nu + 1) / 2 + mu` is the unordered factor pair `mu <= nu` (Lee et al. 2021 Eq. (29),
//! zero-based). The loader proves the payload matches its pinned SHA-256 and that `lambda`,
//! `lambda_lanes` and `identity` recomputed exactly from it equal the values in `spec.json`.
mod payload;

use super::{EncodingSpec, Exact, Monomial, Network, Rotated, RotatedPart, SystemOp};
use num_bigint::BigInt;
use std::path::Path;
use std::sync::Arc;

pub use payload::parse_payload;

pub struct ThcSpec {
    pub id: String,
    /// Spatial orbitals `N`.
    pub n: usize,
    /// THC rank `M` (number of factors `chi_mu`).
    pub m: usize,
    /// Rotation bits: network angle `a` means `2 pi a / 2^beta`.
    pub beta: u8,
    pub ecore: f64,
    /// Eigenvalues of `T'` and their networks.
    pub t: Vec<f64>,
    pub t_nets: Vec<Arc<Network>>,
    /// The factor networks: `chi_nets[mu]` maps spatial orbital 0 to `chi_mu`.
    pub chi_nets: Vec<Arc<Network>>,
    /// `zeta` in pair order: entry `nu (nu + 1) / 2 + mu` is `zeta_{mu nu}`, `mu <= nu`.
    pub zeta_tri: Vec<f64>,
    /// Sum of |coefficient| over the spec's terms.
    pub lambda: Exact,
    /// `lambda` plus the identity lanes' weight `sum_mu |zeta_mumu| / 4`: Lee et al.'s Eq. (20)
    /// form, and the `lambda_decl` of `thc_pair::build`.
    pub lambda_lanes: Exact,
    pub identity: Exact,
    pub sha: [u8; 32],
}

/// What a pair index names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pair {
    OneBody {
        k: usize,
    },
    /// The unordered factor pair `mu <= nu`.
    TwoBody {
        mu: usize,
        nu: usize,
    },
}

fn exact(x: f64) -> Exact {
    Exact::from_f64(x).unwrap_or_else(|_| Exact::zero())
}

/// Exact `sum f(x_i)` of f64 values as one dyadic (a common exponent, no gcd per term).
fn dyadic_sum(xs: impl Iterator<Item = f64>) -> Exact {
    const E: u32 = 1074 + 64;
    let mut acc = BigInt::from(0);
    for x in xs {
        if x == 0.0 || !x.is_finite() {
            continue;
        }
        let bits = x.to_bits();
        let neg = bits >> 63 == 1;
        let exp = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
        let frac = bits & ((1u64 << 52) - 1);
        let (mant, e) = if exp == 0 {
            (frac, -1074)
        } else {
            (frac | (1u64 << 52), exp - 1075)
        };
        // x = mant 2^e = (mant 2^(e + E)) / 2^E, with e + E >= 64 > 0.
        let shift = u32::try_from(e + i64::from(E)).unwrap_or(0);
        let v = BigInt::from(mant) << shift;
        if neg {
            acc -= v;
        } else {
            acc += v;
        }
    }
    Exact::dyadic(acc, E)
}

/// `(nu, mu)` of triangular index `r = nu (nu + 1) / 2 + mu`, `mu <= nu`.
#[must_use]
pub fn tri_decode(r: u64) -> (u64, u64) {
    let mut nu = ((8.0 * r as f64 + 1.0).sqrt() as u64).saturating_sub(1) / 2;
    while (nu + 1) * (nu + 2) / 2 <= r {
        nu += 1;
    }
    while nu * (nu + 1) / 2 > r {
        nu -= 1;
    }
    (nu, r - nu * (nu + 1) / 2)
}

impl ThcSpec {
    #[must_use]
    pub fn pair_count(&self) -> u64 {
        (self.n + self.m * (self.m + 1) / 2) as u64
    }

    /// The pair index of factor pair `(mu, nu)` (either order).
    #[must_use]
    pub fn pair_index(&self, mu: usize, nu: usize) -> u64 {
        let (lo, hi) = if mu <= nu { (mu, nu) } else { (nu, mu) };
        (self.n + hi * (hi + 1) / 2 + lo) as u64
    }

    #[must_use]
    pub fn pair(&self, idx: u64) -> Option<Pair> {
        if idx < self.n as u64 {
            return Some(Pair::OneBody { k: idx as usize });
        }
        if idx >= self.pair_count() {
            return None;
        }
        let (nu, mu) = tri_decode(idx - self.n as u64);
        Some(Pair::TwoBody {
            mu: mu as usize,
            nu: nu as usize,
        })
    }

    /// `zeta_{mu nu}` (symmetric).
    #[must_use]
    pub fn zeta(&self, mu: usize, nu: usize) -> f64 {
        let (lo, hi) = if mu <= nu { (mu, nu) } else { (nu, mu) };
        self.zeta_tri[hi * (hi + 1) / 2 + lo]
    }

    /// Every network in one index space, for lookup tables: `e < N` is the one-body eigenvector
    /// `t_nets[e]`, `e = N + mu` is the factor network `chi_nets[mu]` (`N + M` networks).
    #[must_use]
    pub fn network(&self, e: usize) -> Option<&Arc<Network>> {
        if e < self.n {
            self.t_nets.get(e)
        } else {
            self.chi_nets.get(e - self.n)
        }
    }

    /// The networks a pair's lanes apply, in the `network` index space, for the `swap = false`
    /// order (the first factor is the leftmost, so it acts last): one-body `(k, None)`,
    /// two-body `(N + mu, Some(N + nu))`. With `swap = true` the two-body order is reversed.
    #[must_use]
    pub fn pair_networks(&self, p: Pair) -> (usize, Option<usize>) {
        match p {
            Pair::OneBody { k } => (k, None),
            Pair::TwoBody { mu, nu } => (self.n + mu, Some(self.n + nu)),
        }
    }

    /// `|c_t|` of every term in the pair (all terms of one pair have the same magnitude):
    /// `|t_k| / 2` one-body, `|zeta_{mu nu}| / 8` two-body.
    #[must_use]
    pub fn term_magnitude(&self, p: Pair) -> Exact {
        match p {
            Pair::OneBody { k } => exact(self.t[k]).abs().mul(&Exact::dyadic(1.into(), 1)),
            Pair::TwoBody { mu, nu } => exact(self.zeta(mu, nu))
                .abs()
                .mul(&Exact::dyadic(1.into(), 3)),
        }
    }

    /// Whether the pair's terms carry a minus sign (`t_k > 0`, or `zeta_{mu nu} < 0`).
    #[must_use]
    pub fn negative(&self, p: Pair) -> bool {
        match p {
            Pair::OneBody { k } => self.t[k] > 0.0,
            Pair::TwoBody { mu, nu } => self.zeta(mu, nu) < 0.0,
        }
    }

    /// Number of spec terms in a pair: one-body 2 (spins), `mu < nu` 8 (spins x order),
    /// `mu = nu` 2 (`s1 != s2`, both orders; the two orders are one operator but two terms).
    #[must_use]
    pub fn terms_in(p: Pair) -> u64 {
        match p {
            Pair::TwoBody { mu, nu } if mu != nu => 8,
            _ => 2,
        }
    }

    /// The system operator of the pair at spins `(s1, s2)` and swap bit `swap`, sign included.
    /// Two-body: `Z(chi_mu, s1) Z(chi_nu, s2)` for `swap = false` and the reversed product
    /// `Z(chi_nu, s2) Z(chi_mu, s1)` (its adjoint) for `swap = true`. `None` for `mu = nu`,
    /// `s1 = s2` (`Z^2 = 1`, not a term). One-body terms ignore `s2` and `swap`.
    #[must_use]
    pub fn term_op(&self, p: Pair, s1: u8, s2: u8, swap: bool) -> Option<SystemOp> {
        let phase_neg = u8::from(self.negative(p));
        match p {
            Pair::OneBody { k } => Some(SystemOp::Rotated(Rotated {
                phase: (3 + 2 * phase_neg) % 4, // coefficient -t_k / 2, Z = -i g g
                parts: vec![z_part(&self.t_nets[k], s1)],
            })),
            Pair::TwoBody { mu, nu } => {
                if mu == nu && s1 == s2 {
                    return None;
                }
                let a = z_part(&self.chi_nets[mu], s1);
                let b = z_part(&self.chi_nets[nu], s2);
                Some(SystemOp::Rotated(Rotated {
                    phase: (2 + 2 * phase_neg) % 4,
                    parts: if swap { vec![b, a] } else { vec![a, b] },
                }))
            }
        }
    }

    /// What a `thc-pair-alias-v1` lane of this pair applies: `term_op`, or for the identity
    /// combination (`mu = nu`, `s1 = s2`) the identity times the pair's sign, `sign(zeta_mumu) I`
    /// (what `sign Z(chi_mu, s) Z(chi_mu, s)` is). Identity lanes only shift the encoded operator
    /// by a known multiple of `I`.
    #[must_use]
    pub fn lane_op(&self, p: Pair, s1: u8, s2: u8, swap: bool) -> SystemOp {
        self.term_op(p, s1, s2, swap).unwrap_or_else(|| {
            SystemOp::Monomial(Monomial {
                phase: 2 * u8::from(self.negative(p)),
                majoranas: Vec::new(),
            })
        })
    }

    /// Exact `(lambda, lambda_lanes, identity)`:
    /// `lambda = sum_k |t_k| + sum_{mu<nu} |zeta| + sum_mu |zeta_mumu| / 4`,
    /// `lambda_lanes = lambda + sum_mu |zeta_mumu| / 4`,
    /// `identity = ecore + sum_k t_k - sum_{mu<nu} zeta - sum_mu zeta_mumu / 4`.
    #[must_use]
    pub fn exact_lambda_identity(ecore: f64, t: &[f64], zeta_tri: &[f64]) -> (Exact, Exact, Exact) {
        let (mut off, mut diag) = (Vec::new(), Vec::new());
        let mut r = 0usize;
        for nu in 0.. {
            if r >= zeta_tri.len() {
                break;
            }
            let row = &zeta_tri[r..(r + nu + 1).min(zeta_tri.len())];
            off.extend_from_slice(&row[..row.len().min(nu)]);
            diag.extend(row.get(nu).copied());
            r += nu + 1;
        }
        let quarter = Exact::dyadic(1.into(), 2);
        let lam_t = dyadic_sum(t.iter().map(|x| x.abs()));
        let off_abs = dyadic_sum(off.iter().map(|x| x.abs()));
        let diag_abs4 = dyadic_sum(diag.iter().map(|x| x.abs())).mul(&quarter);
        let lambda = lam_t.add(&off_abs).add(&diag_abs4);
        let lambda_lanes = lambda.add(&diag_abs4);
        let identity = exact(ecore)
            .add(&dyadic_sum(t.iter().copied()))
            .sub(&dyadic_sum(off.iter().copied()))
            .sub(&dyadic_sum(diag.iter().copied()).mul(&quarter));
        (lambda, lambda_lanes, identity)
    }
}

/// `Z(u, s)`: the network's Gaussian conjugating `Z` on spin orbital `s` (spatial orbital 0),
/// `Z_m = -i gamma_{2m} gamma_{2m+1}` (the `-i` is in the caller's phase).
fn z_part(net: &Arc<Network>, spin: u8) -> RotatedPart {
    let m = 2 * u16::from(spin);
    RotatedPart {
        network: Arc::clone(net),
        spin: Some(spin),
        majoranas: vec![m, m + 1],
    }
}

impl EncodingSpec for ThcSpec {
    fn id(&self) -> &str {
        &self.id
    }
    fn encoding(&self) -> &str {
        "thc"
    }
    fn spatial_orbitals(&self) -> usize {
        self.n
    }
    fn lambda(&self) -> Exact {
        self.lambda.clone()
    }
    fn identity(&self) -> Exact {
        self.identity.clone()
    }
    fn payload_sha256(&self) -> [u8; 32] {
        self.sha
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn meta_str<'a>(meta: &'a serde_json::Value, path: &[&str]) -> Result<&'a str, String> {
    let mut v = meta;
    for p in path {
        v = v
            .get(p)
            .ok_or_else(|| format!("spec.json: missing {}", path.join(".")))?;
    }
    v.as_str()
        .ok_or_else(|| format!("spec.json: {} is not a string", path.join(".")))
}

/// Loads `dir/thc.bin` and proves it matches `spec.json` (and `specs/INDEX.json` when present).
///
/// # Errors
/// A hash mismatch, a malformed payload, or `lambda`/`lambda_lanes`/`identity` that differ from
/// the payload's.
pub fn load(dir: &Path, meta: &serde_json::Value) -> Result<Box<dyn EncodingSpec>, String> {
    let id = meta_str(meta, &["id"])?.to_string();
    let file = meta_str(meta, &["payload", "file"])?;
    let bytes = std::fs::read(dir.join(file)).map_err(|e| format!("spec {id}: {e}"))?;
    let spec = parse_payload(&id, &bytes)?;
    let sha = hex::encode(spec.sha);
    if meta_str(meta, &["payload", "sha256"])? != sha {
        return Err(format!(
            "spec {id}: payload sha256 {sha} differs from spec.json"
        ));
    }
    super::df::check_index(dir, &id, &sha)?;
    for (name, v) in [
        ("lambda", &spec.lambda),
        ("lambda_lanes", &spec.lambda_lanes),
        ("identity", &spec.identity),
    ] {
        let want = meta_str(meta, &[name, "exact"])?;
        if want != v.to_string() {
            return Err(format!(
                "spec {id}: {name} {v} recomputed from the payload differs from spec.json {want}"
            ));
        }
    }
    Ok(Box::new(spec))
}
