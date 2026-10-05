//! Contestant code (EDITABLE): the walk step this submission builds.
//!
//! The walk API (spec/DESIGN.md section 4), called only by the untrusted `build_circuit`:
//!
//! - `spec_id() -> &'static str`: the pinned spec under `specs/` this walk implements.
//! - `build(spec: &dyn EncodingSpec, b: &mut Builder) -> Box<dyn LaneMap>`: first call
//!   `b.declare_uniform(u)` with the lane map's `uniform_bits()`, then emit the controlled
//!   block encoding through `b` (control `b.control()`, system `b.system(j)`, uniform
//!   `b.uniform(i)`, ancillas from `b.alloc()`, all returned to `|0>` by `b.free` or `b.hmr`),
//!   and return the lane map declaring which term each uniform value selects.
//! - `family() -> Family`: the declared taxonomy tuple (`taxonomy/taxonomy.json`).
//!
//! This release ships the spectrum-amplified architectures of `sa_low/` (its README lists every
//! lever). This module is compiled only with the `walk` cargo feature (on by default); the
//! trusted `eval_circuit` is built with `--no-default-features`, so no code from here is linked
//! into it.
pub mod common;
pub mod sa_low;
pub mod shared;

use crate::circuit::Builder;
use crate::lanemap::LaneMap;
use crate::spec::EncodingSpec;
use crate::taxonomy::Family;

/// The spec this submission builds: one of the two tracks, `reiher-sa-est-v1` (the default) or
/// `li-sa-est-v1`. Set `FEMOCO_WALK_SPEC` at build time (for example
/// `FEMOCO_WALK_SPEC=li-sa-est-v1 ./benchmark.sh`) to build for the other.
pub const SPEC_ID: &str = match option_env!("FEMOCO_WALK_SPEC") {
    Some(id) => id,
    None => "reiher-sa-est-v1",
};

/// The architecture this submission builds. The default is Low et al. 2025's published step
/// (`sa-low2025`); set `FEMOCO_WALK_ARCH` at build time to build another. New architectures
/// register one arm in `build` and one in `family`.
pub const ARCH: &str = match option_env!("FEMOCO_WALK_ARCH") {
    Some(arch) => arch,
    None => "sa-low2025",
};

/// The spec this submission implements.
#[must_use]
pub fn spec_id() -> &'static str {
    SPEC_ID
}

/// Emits the walk step for the chosen architecture and the spec's encoding.
///
/// # Panics
/// If no architecture of that name exists for the spec's encoding, or no lane map fits it.
pub fn build(spec: &dyn EncodingSpec, b: &mut Builder) -> Box<dyn LaneMap> {
    match (ARCH, spec.encoding()) {
        ("sa-low2025", "sos-sa") => sa_low::build(spec, b, true),
        ("sa-low2025-bothspin", "sos-sa") => sa_low::build(spec, b, false),
        ("sa-lowq", "sos-sa") => sa_low::build_lowq(spec, b),
        ("sa-toff", "sos-sa") => sa_low::build_toff(spec, b, true, sa_low::Tweaks::toff()),
        ("sa-pareto", "sos-sa") => sa_low::build_pareto(spec, b),
        (arch, encoding) => panic!("no walk architecture '{arch}' for encoding '{encoding}'"),
    }
}

/// The declared family of the chosen architecture.
///
/// # Panics
/// If no architecture of that name exists.
#[must_use]
pub fn family() -> Family {
    match (ARCH, "sos-sa") {
        ("sa-low2025", "sos-sa") => sa_low::family(true),
        ("sa-low2025-bothspin", "sos-sa") => sa_low::family(false),
        ("sa-lowq", "sos-sa") => sa_low::family_lowq(),
        ("sa-toff", "sos-sa") => sa_low::family_toff(),
        ("sa-pareto", "sos-sa") => sa_low::family_pareto(),
        (arch, encoding) => panic!("no walk architecture '{arch}' for encoding '{encoding}'"),
    }
}
