//! Lane maps: structured data a submission declares, from harness-supported families.
//! See spec/DESIGN.md section 6. `sparse_sym.rs`, `df_pair.rs` and `thc_pair.rs` are the
//! families of the sparse, DF and THC encodings (no spec of those encodings ships here); each
//! implements `parse` with the signature below.
//! `df_nested.rs` (the nested DF lane map) is one family whose lanes cross a `Reflect`
//! (spec/DESIGN.md section 16): it answers `inner_bits` and `reference_nested`. `sa_nested.rs`
//! (the spectrum-amplified block encoding over an `sos-sa` spec, spec/SPEC-SA.md section 5) is
//! the second such family.
pub mod alias;
pub mod df_nested;
pub mod df_pair;
pub mod sa_nested;
pub mod sparse_sym;
pub mod thc_pair;

use crate::spec::rounding::{Estimate, EstimatedParams};
use crate::spec::{EncodingSpec, Exact, SystemOp};

pub const MAGIC: &[u8; 8] = b"FEMOLMAP";

pub trait LaneMap: Send + Sync {
    /// Trusted SA table view for deterministic coverage and symbolic export.
    fn as_sa_nested(&self) -> Option<&sa_nested::SaNestedMap> {
        None
    }
    /// "alias-v1", "sparse-sym-alias-v1", "df-pair-alias-v1", "df-nested-alias-v1",
    /// "thc-pair-alias-v1" or "sa-nested-alias-v1".
    fn family(&self) -> &str;
    /// Width `u` of the uniform register.
    fn uniform_bits(&self) -> u32;
    /// Declared dyadic normalization.
    fn lambda_decl(&self) -> Exact;
    /// The system operator lane `s` must apply when the control is 1.
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp;
    /// The uniform value a control-1 lane `s` must hold at the end: `s` for every family except
    /// `thc-pair-alias-v1`, whose block encoding also flips the swap bit (the THC encoding; no THC spec ships here
    /// section 6). A family that overrides this must make it an involution on `0..2^u` with
    /// `reference_op(uniform_after(s)) = reference_op(s)^dagger`, so the block encoding
    /// `sum_s |uniform_after(s)><s| (x) M_s` stays Hermitian (spec/DESIGN.md section 6).
    fn uniform_after(&self, s: u64) -> u64 {
        s
    }
    /// Exact `sum_t |c_t - lambda_decl * n_t / 2^u|` against the spec, from the declared data.
    ///
    /// # Errors
    /// Declared data inconsistent with the spec.
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String>;
    /// The estimated rounding procedure (spec/SPEC-SA.md section 14), for specs of the estimated class only:
    /// its exact floor-or-ceiling check of every count and the resolution of every table. Only
    /// `sa-nested-alias-v1` implements it; every other family refuses.
    ///
    /// # Errors
    /// A family with no such procedure, or a lane map that is not a rounding of the spec's weights.
    fn rounding_estimate(
        &self,
        spec: &dyn EncodingSpec,
        params: &EstimatedParams,
    ) -> Result<Estimate, String> {
        let _ = (spec, params);
        Err(format!(
            "lane map family {} has no estimated rounding procedure",
            self.family()
        ))
    }
    /// Alias index bits `k` and keep bits `mu`, for families that have them (facts only).
    fn alias_bits(&self) -> Option<(u32, u32)> {
        None
    }
    /// `lanemap.bin`: `MAGIC`, u16 family length, family bytes, then the family's payload.
    fn to_bytes(&self) -> Vec<u8>;
    /// Nested lane maps only: the inner uniform register as `(first bit, width)`. `None` (every
    /// other family) means the circuit may not contain a `Reflect`.
    fn inner_bits(&self) -> Option<(u32, u32)> {
        None
    }
    /// Nested lane maps only: the operator a control-1 lane must apply when its uniform value is
    /// `s` before the `Reflect` and `after` (inner part replaced by the second-pass value) after
    /// it (spec/DESIGN.md section 16.2).
    fn reference_nested(&self, spec: &dyn EncodingSpec, s: u64, after: u64) -> SystemOp {
        let _ = after;
        self.reference_op(spec, s)
    }
    /// `sa-nested-alias-v1` only: the `sos-ground-cs-v1` bound on the ground-energy error of the
    /// rounded sum of squares, with `g = E_up - E_SOS` (spec/SPEC-SA.md section 12). `None` for
    /// every other family; the harness asks only when an sos-sa spec declares that rule.
    fn ground_bound(
        &self,
        spec: &dyn EncodingSpec,
        g: &Exact,
    ) -> Option<Result<sa_nested::GroundBound, String>> {
        let _ = (spec, g);
        None
    }
}

/// Parses `lanemap.bin` and dispatches on its family string.
///
/// # Errors
/// A malformed file or an unknown family.
pub fn parse(bytes: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String> {
    let rest = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or("lanemap.bin: bad magic")?;
    if rest.len() < 2 {
        return Err("lanemap.bin: truncated".into());
    }
    let len = usize::from(u16::from_le_bytes([rest[0], rest[1]]));
    let family = rest
        .get(2..2 + len)
        .ok_or("lanemap.bin: truncated family")?;
    let payload = &rest[2 + len..];
    match std::str::from_utf8(family).map_err(|e| e.to_string())? {
        "alias-v1" => alias::parse(payload, spec),
        "sparse-sym-alias-v1" => sparse_sym::parse(payload, spec),
        "df-pair-alias-v1" => df_pair::parse(payload, spec),
        "df-nested-alias-v1" => df_nested::parse(payload, spec),
        "thc-pair-alias-v1" => thc_pair::parse(payload, spec),
        sa_nested::FAMILY => sa_nested::parse(payload, spec),
        other => Err(format!("lanemap.bin: unknown family {other}")),
    }
}
