//! Encoding specs: shared types and the loader. See spec/DESIGN.md section 7.
//! `sparse.rs`, `df.rs` and `thc.rs` load the sparse, DF and THC encodings (no spec of those
//! encodings ships here); each implements `load` with the signature below. `sa.rs` is the
//! spectrum-amplified sum-of-squares encoding `sos-sa` (spec/SPEC-SA.md).
pub mod df;
pub mod exact;
pub mod rounding;
pub mod sa;
pub mod sparse;
pub mod thc;

pub use exact::Exact;
use std::path::Path;

pub type TermId = u64;

/// `i^phase * gamma_{m_0} ... gamma_{m_{d-1}}` on the Jordan-Wigner Majoranas, indices strictly
/// increasing, phase chosen so the product is Hermitian. The identity is the empty product.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Monomial {
    pub phase: u8,
    pub majoranas: Vec<u16>,
}

/// A network of fermionic Givens rotations (spec/DESIGN.md section 15): `rotations[j] = (p, q, a)` is
/// `G_{p,q}(2 pi a / 2^beta)` on modes `p < q`, applied in order (the first entry acts first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub beta: u8,
    pub rotations: Vec<(u16, u16, u32)>,
}

/// `G m G^dagger`: a Majorana monomial `m` (spin-orbital Majorana indices, strictly increasing,
/// `m.phase` ignored) conjugated by the Gaussian unitary `G` of `network`. With `spin = Some(s)`
/// the network's indices are spatial orbitals and act on spin orbitals `2p + s`; with `None`
/// they are spin orbitals.
#[derive(Clone, Debug, PartialEq)]
pub struct RotatedPart {
    pub network: std::sync::Arc<Network>,
    pub spin: Option<u8>,
    pub majoranas: Vec<u16>,
}

/// `i^phase * prod_j (G_j m_j G_j^dagger)` in the order given (spec/DESIGN.md section 15).
/// Compared by the fermionic-Gaussian tracker (`crate::sim::gaussian`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rotated {
    pub phase: u8,
    pub parts: Vec<RotatedPart>,
}

/// The system action a spec assigns to one term.
#[derive(Clone, Debug, PartialEq)]
pub enum SystemOp {
    Monomial(Monomial),
    Rotated(Rotated),
}

impl SystemOp {
    /// The Majorana monomial, when this op is one. The `Rotated` variant returns
    /// `None` here and is compared by the Givens tracker instead (`crate::sim::tracker`).
    #[must_use]
    pub fn as_monomial(&self) -> Option<&Monomial> {
        match self {
            Self::Monomial(m) => Some(m),
            Self::Rotated(_) => None,
        }
    }
}

pub trait EncodingSpec: Send + Sync {
    fn id(&self) -> &str;
    /// "sparse", "df", "thc" or "sos-sa".
    fn encoding(&self) -> &str;
    fn spatial_orbitals(&self) -> usize;
    fn system_qubits(&self) -> usize {
        2 * self.spatial_orbitals()
    }
    /// Sum of |coefficient| over the spec's terms.
    fn lambda(&self) -> Exact;
    /// Identity coefficient (classical energy offset).
    fn identity(&self) -> Exact;
    fn payload_sha256(&self) -> [u8; 32];
    /// Every term, when the spec is small enough to flatten (used by `alias-v1` and tests).
    fn flat_terms(&self) -> Option<Vec<(Exact, SystemOp)>> {
        None
    }
    /// Lets a lane map reach its own spec type's internals.
    fn as_any(&self) -> &dyn std::any::Any;
    /// How a lane map's rounding error is judged (spec/SPEC-SA.md section 14): `Rigorous` (the exact 1-norm at
    /// most 0.1 mHa) for every spec except the two pinned `*-sa-est-v1` specs.
    fn rounding_class(&self) -> rounding::RoundingClass {
        rounding::RoundingClass::Rigorous
    }
}

/// Loads `<root>/specs/<id>/spec.json` and dispatches on its `encoding` field.
///
/// # Errors
/// An id that is not `[a-z0-9-]+`, a missing or malformed spec, or an unknown encoding.
pub fn load(root: &Path, id: &str) -> Result<Box<dyn EncodingSpec>, String> {
    // The id comes from the untrusted family.out.json: it must name a directory directly under
    // specs/ (no path separators or `..`, which could point at contestant-written files).
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(format!("spec id {id:?} is not of the form [a-z0-9-]+"));
    }
    let dir = root.join("specs").join(id);
    let text =
        std::fs::read_to_string(dir.join("spec.json")).map_err(|e| format!("spec {id}: {e}"))?;
    let meta: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("spec {id}: {e}"))?;
    // A `rounding_class` other than the default is defined for the pinned sos-sa ids only
    // (spec/SPEC-SA.md section 14); anywhere else the spec is refused rather than silently read as rigorous.
    let class = rounding::from_meta(id, &meta)?;
    let encoding = meta.get("encoding").and_then(serde_json::Value::as_str);
    if class != rounding::RoundingClass::Rigorous && encoding != Some(sa::ENCODING) {
        return Err(format!(
            "spec {id}: a rounding_class is defined for sos-sa specs only"
        ));
    }
    match encoding {
        Some("sparse") => sparse::load(&dir, &meta),
        Some("df") => df::load(&dir, &meta),
        Some("thc") => thc::load(&dir, &meta),
        Some(sa::ENCODING) => sa::load(&dir, &meta),
        other => Err(format!("spec {id}: unknown encoding {other:?}")),
    }
}
