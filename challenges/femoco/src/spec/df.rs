//! The df spec: double factorization with quantized Givens networks (the DF encoding; no DF spec ships here).
//!
//!
//! The operator is `H = identity + sum_{k,s} (-t_k / 2) Z(w_k, s)
//! + (1/8) sum_l sum_{(k1,s1) != (k2,s2)} e_{l k1} e_{l k2} Z(u_{l k1}, s1) Z(u_{l k2}, s2)`,
//! where `Z(u, s) = G_u Z_{mode s} G_u^dagger` and `G_u` is the quantized Givens network that
//! maps spatial orbital 0 to `u`. Terms are grouped in *pairs*: pair `k < N` is the one-body
//! eigenvector `k`, then leaf `l` contributes `Xi_l^2` pairs `(k1, k2)`. The loader proves the
//! payload matches its pinned SHA-256 and that `lambda` and `identity` recomputed exactly from
//! it equal the values written in `spec.json`.
mod payload;

use super::{EncodingSpec, Exact, Network, Rotated, RotatedPart, SystemOp};
use std::path::Path;
use std::sync::Arc;

pub use payload::{check_index, parse_payload};

/// Which LCU form of the same operator a df spec states (the DF encoding; no DF spec ships here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DfForm {
    /// The flat pair LCU: `lambda = lambda_T + sum_l (S_l^2/2 - Q_l/4)` (`*-df-v1`).
    Flat,
    /// The nested Chebyshev form: `lambda = lambda_T + sum_l S_l^2/4` (`*-df-nested-v1`).
    Nested,
}

/// One leaf of the second factorization: kept eigenvalues and their networks.
pub struct Leaf {
    pub e: Vec<f64>,
    pub nets: Vec<Arc<Network>>,
}

pub struct DfSpec {
    pub id: String,
    /// Spatial orbitals `N`.
    pub n: usize,
    /// Rotation bits: network angle `a` means `2 pi a / 2^beta`.
    pub beta: u8,
    pub ecore: f64,
    /// Eigenvalues of `T'` and their networks.
    pub t: Vec<f64>,
    pub t_nets: Vec<Arc<Network>>,
    pub leaves: Vec<Leaf>,
    /// `offsets[l]` is leaf `l`'s first pair; `offsets[L]` is the pair count.
    pub offsets: Vec<u64>,
    pub lambda: Exact,
    pub identity: Exact,
    pub sha: [u8; 32],
    /// Flat unless the spec's `spec.json` says `"form": "nested"`.
    pub form: DfForm,
}

/// What a pair index names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pair {
    OneBody { k: usize },
    TwoBody { l: usize, k1: usize, k2: usize },
}

fn exact(x: f64) -> Exact {
    Exact::from_f64(x).unwrap_or_else(|_| Exact::zero())
}

impl DfSpec {
    #[must_use]
    pub fn pair_count(&self) -> u64 {
        self.offsets.last().copied().unwrap_or(0)
    }

    #[must_use]
    pub fn pair(&self, idx: u64) -> Option<Pair> {
        if idx < self.n as u64 {
            return Some(Pair::OneBody { k: idx as usize });
        }
        if idx >= self.pair_count() {
            return None;
        }
        let l = self.offsets.partition_point(|&o| o <= idx) - 1;
        let xi = self.leaves[l].e.len() as u64;
        let r = idx - self.offsets[l];
        Some(Pair::TwoBody {
            l,
            k1: (r / xi) as usize,
            k2: (r % xi) as usize,
        })
    }

    /// `|c_t|` of every term in the pair (all terms of one pair have the same magnitude):
    /// `|t_k| / 2` one-body, `|e_{l k1} e_{l k2}| / 8` two-body.
    #[must_use]
    pub fn term_magnitude(&self, p: Pair) -> Exact {
        match p {
            Pair::OneBody { k } => exact(self.t[k]).abs().mul(&Exact::dyadic(1.into(), 1)),
            Pair::TwoBody { l, k1, k2 } => {
                let e = &self.leaves[l].e;
                exact(e[k1])
                    .mul(&exact(e[k2]))
                    .abs()
                    .mul(&Exact::dyadic(1.into(), 3))
            }
        }
    }

    /// The system operator of the term at spins `(s1, s2)`, sign included; `None` for a
    /// same-orbital same-spin two-body combination (`Z^2 = 1`, not a term). One-body terms
    /// ignore `s2`.
    #[must_use]
    pub fn term_op(&self, p: Pair, s1: u8, s2: u8) -> Option<SystemOp> {
        match p {
            Pair::OneBody { k } => {
                let neg = self.t[k] > 0.0; // coefficient -t_k / 2
                Some(SystemOp::Rotated(Rotated {
                    phase: (3 + 2 * u8::from(neg)) % 4,
                    parts: vec![z_part(&self.t_nets[k], s1)],
                }))
            }
            Pair::TwoBody { l, k1, k2 } => {
                if k1 == k2 && s1 == s2 {
                    return None;
                }
                let leaf = &self.leaves[l];
                let neg = (leaf.e[k1] < 0.0) != (leaf.e[k2] < 0.0);
                Some(SystemOp::Rotated(Rotated {
                    phase: (2 + 2 * u8::from(neg)) % 4,
                    parts: vec![z_part(&leaf.nets[k1], s1), z_part(&leaf.nets[k2], s2)],
                }))
            }
        }
    }

    /// Number of spec terms in a pair (one-body 2, two-body 4, or 2 when `k1 = k2`).
    #[must_use]
    pub fn terms_in(p: Pair) -> u64 {
        match p {
            Pair::TwoBody { k1, k2, .. } if k1 != k2 => 4,
            _ => 2,
        }
    }

    /// The same operator in nested form (the nested DF form; no DF spec ships here): `lambda` and `identity`
    /// become `lambda_T + sum_l S_l^2 / 4` and `ecore + sum_k t_k + sum_l (S_l^2/4 - A_l^2/2)`.
    #[must_use]
    pub fn into_nested(mut self) -> Self {
        (self.lambda, self.identity) =
            Self::exact_nested_lambda_identity(self.ecore, &self.t, &self.leaves);
        self.form = DfForm::Nested;
        self
    }

    /// Exact nested-form `(lambda, identity)`, the nested DF form; no DF spec ships here.
    #[must_use]
    pub fn exact_nested_lambda_identity(ecore: f64, t: &[f64], leaves: &[Leaf]) -> (Exact, Exact) {
        let sum = |f: &dyn Fn(f64) -> Exact, v: &[f64]| {
            v.iter().fold(Exact::zero(), |a, &x| a.add(&f(x)))
        };
        let mut lam = sum(&|x| exact(x).abs(), t);
        let mut ident = exact(ecore).add(&sum(&exact, t));
        let (half, quarter) = (Exact::dyadic(1.into(), 1), Exact::dyadic(1.into(), 2));
        for leaf in leaves {
            let s = sum(&|x| exact(x).abs(), &leaf.e);
            let a = sum(&exact, &leaf.e);
            let s4 = s.mul(&s).mul(&quarter);
            lam = lam.add(&s4);
            ident = ident.add(&s4).sub(&a.mul(&a).mul(&half));
        }
        (lam, ident)
    }

    /// `lambda_T = sum_k |t_k|`, exactly.
    #[must_use]
    pub fn lambda_t(&self) -> Exact {
        self.t
            .iter()
            .fold(Exact::zero(), |a, &x| a.add(&exact(x).abs()))
    }

    /// `S_l^2 / 4` for leaf `l`, exactly (its Chebyshev weight).
    #[must_use]
    pub fn leaf_weight(&self, l: usize) -> Exact {
        let s = self.leaves[l]
            .e
            .iter()
            .fold(Exact::zero(), |a, &x| a.add(&exact(x).abs()));
        s.mul(&s).mul(&Exact::dyadic(1.into(), 2))
    }

    /// The one-body term `(k, s)`: `-sign(t_k) Z(w_k, s)` (its coefficient is `|t_k| / 2`).
    #[must_use]
    pub fn one_body_op(&self, k: usize, spin: u8) -> SystemOp {
        let neg = self.t[k] > 0.0;
        SystemOp::Rotated(Rotated {
            phase: (3 + 2 * u8::from(neg)) % 4,
            parts: vec![z_part(&self.t_nets[k], spin)],
        })
    }

    /// Leaf `l`'s inner operator `M_lj = sign(e_lk) Z(u_lk, s)` for `j = (k, s)`
    /// (the nested DF form; no DF spec ships here).
    #[must_use]
    pub fn inner_op(&self, l: usize, k: usize, spin: u8) -> SystemOp {
        let leaf = &self.leaves[l];
        SystemOp::Rotated(Rotated {
            phase: if leaf.e[k] < 0.0 { 1 } else { 3 },
            parts: vec![z_part(&leaf.nets[k], spin)],
        })
    }

    /// Exact `lambda = sum_k |t_k| + sum_l (S_l^2 / 2 - Q_l / 4)` and
    /// `identity = ecore + sum_k t_k + sum_l (-A_l^2 / 2 + Q_l / 4)`.
    #[must_use]
    pub fn exact_lambda_identity(ecore: f64, t: &[f64], leaves: &[Leaf]) -> (Exact, Exact) {
        let sum = |f: &dyn Fn(f64) -> Exact, v: &[f64]| {
            v.iter().fold(Exact::zero(), |a, &x| a.add(&f(x)))
        };
        let mut lam = sum(&|x| exact(x).abs(), t);
        let mut ident = exact(ecore).add(&sum(&exact, t));
        let (half, quarter) = (Exact::dyadic(1.into(), 1), Exact::dyadic(1.into(), 2));
        for leaf in leaves {
            let s = sum(&|x| exact(x).abs(), &leaf.e);
            let a = sum(&exact, &leaf.e);
            let q = sum(&|x| exact(x).mul(&exact(x)), &leaf.e);
            let q4 = q.mul(&quarter);
            lam = lam.add(&s.mul(&s).mul(&half)).sub(&q4);
            ident = ident.sub(&a.mul(&a).mul(&half)).add(&q4);
        }
        (lam, ident)
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

impl EncodingSpec for DfSpec {
    fn id(&self) -> &str {
        &self.id
    }
    fn encoding(&self) -> &str {
        "df"
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

/// Loads `dir/df.bin` and proves it matches `spec.json` (and `specs/INDEX.json` when present).
///
/// # Errors
/// A hash mismatch, a malformed payload, or `lambda`/`identity` that differ from the payload's.
pub fn load(dir: &Path, meta: &serde_json::Value) -> Result<Box<dyn EncodingSpec>, String> {
    let id = meta_str(meta, &["id"])?.to_string();
    let file = meta_str(meta, &["payload", "file"])?;
    let bytes = std::fs::read(dir.join(file)).map_err(|e| format!("spec {id}: {e}"))?;
    let spec = match meta.get("form").and_then(serde_json::Value::as_str) {
        None | Some("flat") => parse_payload(&id, &bytes)?,
        Some("nested") => parse_payload(&id, &bytes)?.into_nested(),
        Some(other) => return Err(format!("spec {id}: unknown df form {other:?}")),
    };
    let sha = hex::encode(spec.sha);
    if meta_str(meta, &["payload", "sha256"])? != sha {
        return Err(format!(
            "spec {id}: payload sha256 {sha} differs from spec.json"
        ));
    }
    payload::check_index(dir, &id, &sha)?;
    for (name, v) in [("lambda", &spec.lambda), ("identity", &spec.identity)] {
        let want = meta_str(meta, &[name, "exact"])?;
        if want != v.to_string() {
            return Err(format!(
                "spec {id}: {name} {v} recomputed from the payload differs from spec.json {want}"
            ));
        }
    }
    Ok(Box::new(spec))
}
