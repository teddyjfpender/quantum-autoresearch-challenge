//! The sparse encoding spec (Lee et al. 2021, arXiv:2011.03494, Appendix A).
//! (spec/DESIGN.md section 7; no sparse spec ships here).
//!
//! A spec is the unique integrals plus the expansion rule of [`rule`]: entry `e` and five
//! expansion bits `x` give the Hermitian Majorana monomial `M_{e,x}` with weight `w_e / 32`, and
//! `H = identity + sum_{e,x} (w_e / 32) M_{e,x}` (up to the documented truncation).
pub mod dyadic;
pub mod payload;
pub mod rule;

use super::{EncodingSpec, Exact, Monomial, SystemOp};
use dyadic::Dyadic;
use rule::{Entry, EXPANSION_BITS};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Specs with at most this many expanded terms can be flattened (`flat_terms`).
const FLAT_LIMIT: usize = 1 << 16;

pub struct SparseSpec {
    id: String,
    spatial_orbitals: usize,
    threshold: f64,
    n_one: usize,
    entries: Vec<Entry>,
    weights: Vec<Dyadic>,
    lambda: Dyadic,
    identity: Dyadic,
    sha256: [u8; 32],
}

fn meta_str<'a>(meta: &'a serde_json::Value, path: &[&str]) -> Result<&'a str, String> {
    path.iter()
        .try_fold(meta, |v, k| v.get(k))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("spec.json: missing {}", path.join(".")))
}

impl SparseSpec {
    /// Builds a spec from payload bytes and checks them against `spec.json`: the payload's
    /// SHA-256, orbital and entry counts, and the exact `lambda`, which is recomputed here from
    /// the entries (so a loaded spec's `lambda` is the 1-norm of the LCU it expands to).
    ///
    /// # Errors
    /// Any mismatch between the payload and `spec.json`, or a malformed payload.
    pub fn from_parts(meta: &serde_json::Value, bytes: &[u8]) -> Result<Self, String> {
        let id = meta_str(meta, &["id"])?.to_string();
        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        if hex::encode(sha256) != meta_str(meta, &["payload", "sha256"])? {
            return Err(format!(
                "spec {id}: payload sha256 does not match spec.json"
            ));
        }
        let p = payload::parse(bytes)?;
        let want = |k: &str| meta.get(k).and_then(serde_json::Value::as_u64);
        if want("spatial_orbitals") != u64::try_from(p.spatial_orbitals).ok()
            || want("terms") != u64::try_from(p.entries.len()).ok()
        {
            return Err(format!(
                "spec {id}: orbital or entry count differs from spec.json"
            ));
        }
        let weights: Vec<Dyadic> = p.entries.iter().map(rule::weight).collect();
        let lambda = dyadic::sum(weights.iter());
        if lambda != Dyadic::parse(meta_str(meta, &["lambda", "exact"])?)? {
            return Err(format!(
                "spec {id}: lambda recomputed from the payload differs from spec.json"
            ));
        }
        let identity = Dyadic::parse(meta_str(meta, &["identity", "exact"])?)?;
        Ok(Self {
            id,
            spatial_orbitals: p.spatial_orbitals,
            threshold: p.threshold,
            n_one: p.n_one,
            entries: p.entries,
            weights,
            lambda,
            identity,
            sha256,
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Number of one-body entries (they come first; entry ids `0..n_one`).
    #[must_use]
    pub fn one_body_entries(&self) -> usize {
        self.n_one
    }

    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Exact `w_e` of entry `e` (each of its 32 terms carries `w_e / 32`).
    #[must_use]
    pub fn weight(&self, e: usize) -> &Dyadic {
        &self.weights[e]
    }

    /// The monomial of term `(e, x)`.
    #[must_use]
    pub fn term(&self, e: usize, x: u8) -> Monomial {
        rule::expand(&self.entries[e], x)
    }

    #[must_use]
    pub fn lambda_dyadic(&self) -> &Dyadic {
        &self.lambda
    }
}

impl EncodingSpec for SparseSpec {
    fn id(&self) -> &str {
        &self.id
    }
    fn encoding(&self) -> &str {
        "sparse"
    }
    fn spatial_orbitals(&self) -> usize {
        self.spatial_orbitals
    }
    fn lambda(&self) -> Exact {
        self.lambda.to_exact()
    }
    fn identity(&self) -> Exact {
        self.identity.to_exact()
    }
    fn payload_sha256(&self) -> [u8; 32] {
        self.sha256
    }
    /// Every `(e, x)` term with weight `w_e / 32`, entry-major, `x` minor. Identical monomials
    /// from different terms are not merged.
    fn flat_terms(&self) -> Option<Vec<(Exact, SystemOp)>> {
        let per = 1usize << EXPANSION_BITS;
        if self.entries.len() * per > FLAT_LIMIT {
            return None;
        }
        let one = num_bigint::BigInt::from(1);
        let terms = (0..self.entries.len())
            .flat_map(|e| (0..per).map(move |x| (e, x)))
            .map(|(e, x)| {
                let w = self.weights[e].scale(&one, EXPANSION_BITS).to_exact();
                (
                    w,
                    SystemOp::Monomial(self.term(e, u8::try_from(x).unwrap_or(0))),
                )
            })
            .collect();
        Some(terms)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Loads `<dir>/spec.json`'s payload (`payload.file`) and checks it (see [`SparseSpec::from_parts`]).
///
/// # Errors
/// A missing or malformed payload, or one that disagrees with `spec.json`.
pub fn load(dir: &Path, meta: &serde_json::Value) -> Result<Box<dyn EncodingSpec>, String> {
    Ok(Box::new(load_sparse(dir, meta)?))
}

/// [`load`] with the concrete type, for baselines and tests.
///
/// # Errors
/// As [`load`].
pub fn load_sparse(dir: &Path, meta: &serde_json::Value) -> Result<SparseSpec, String> {
    let file = meta_str(meta, &["payload", "file"])?;
    if file.contains('/') || file.contains("..") {
        return Err(format!("spec.json: bad payload file name {file:?}"));
    }
    let bytes =
        std::fs::read(dir.join(file)).map_err(|e| format!("{}: {e}", dir.join(file).display()))?;
    SparseSpec::from_parts(meta, &bytes)
}
