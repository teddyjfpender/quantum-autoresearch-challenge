//! Lane simulator (spec/DESIGN.md sections 6, 9, 15 and 16).
//!
//! `compile` checks the op stream statically and lowers it; `lanes` executes it bit-sliced on
//! 64 lanes per word with the system register carried as a Pauli frame; `validate` runs the
//! sampled lanes in parallel and applies the per-lane checks; `jw` is the exact Jordan-Wigner
//! conversion every encoding compares system operators through; `tracker` is the plug-in point for
//! the fermionic-Gaussian tracking of `Givens`.
pub mod compile;
pub mod gaussian;
pub mod jw;
pub mod lanes;
pub mod liveness;
pub mod nested;
pub mod tracker;
pub mod validate;

pub use compile::{compile, compile_nested, compile_sa, Compiled, Layout};
pub use jw::{monomial_to_frame, Frame};
pub use nested::Inner;
pub use tracker::{LaneFrame, LaneTracker, TrackerFactory};

/// The `Givens` tracker for `spec`: the fermionic-Gaussian tracker at the df, thc or sos-sa spec's
/// rotation precision, and none for any other encoding (so `Givens` is rejected there with "Givens
/// needs the df tracker (not in this build)").
#[must_use]
pub fn givens_tracker(spec: &dyn crate::spec::EncodingSpec) -> Option<&'static dyn TrackerFactory> {
    let any = spec.as_any();
    let beta = if let Some(df) = any.downcast_ref::<crate::spec::df::DfSpec>() {
        df.beta
    } else if let Some(sa) = any.downcast_ref::<crate::spec::sa::SaSpec>() {
        // A tapered sos-sa spec (spec/SPEC-SA.md section 13) gets its per-position tracker.
        if let Some(w) = &sa.widths {
            return gaussian::tapered_factory(sa.beta, w).map(|f| f as &dyn TrackerFactory);
        }
        sa.beta
    } else {
        any.downcast_ref::<crate::spec::thc::ThcSpec>()?.beta
    };
    gaussian::factory(u32::from(beta)).map(|f| f as &dyn TrackerFactory)
}
