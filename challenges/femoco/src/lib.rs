//! FeMoco walk-step challenge harness.
//!
//! TRUSTED: every module except `walk`. `walk` is the contestant's editable code, compiled
//! only with the `walk` feature (default on); the trusted `eval_circuit` is built without it so
//! no contestant code (not even a link-section initializer) is linked into it.
//! See spec/DESIGN.md for the contract between them.
pub mod circuit;
pub mod equiv;
pub mod facts;
pub mod fastsim;
pub mod fiat_shamir;
pub mod lanemap;
pub mod score;
pub mod sim;
pub mod spec;
pub mod taxonomy;
#[cfg(feature = "walk")]
pub mod walk;
