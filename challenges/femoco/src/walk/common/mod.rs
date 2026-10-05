//! Helpers every baseline uses: unary iteration, reversible arithmetic and table lookups.
//! Moved here from `sparse/` and `df/`, which had carried copies of the same code; the op
//! streams they emit are unchanged.
pub mod arith;
pub mod arith_gated;
pub mod lookup;
pub mod unary;
