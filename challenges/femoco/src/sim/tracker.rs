//! The plug-in point for a per-lane system tracker beyond the Pauli frame (the
//! fermionic-Gaussian tracker for `Givens`, `src/sim/gaussian.rs`).
//!
//! The bit-sliced simulator carries each lane's system operator as
//! `w^phase * D * X^x Z^z` with `D = prod_q S_q^{s_q}`. When a `Givens` executes on a lane, the
//! simulator hands the lane's current `D X^x Z^z` (phase excluded: it stays in the shared Z/8
//! planes) to that lane's tracker, which folds it into its own representation, and the
//! simulator resets the lane's frame to the identity. At the end of the lane, a lane that has a
//! tracker (or whose reference op is not a Majorana monomial) is judged by the tracker's
//! `finish`, given the remaining frame with the full phase.
use crate::spec::SystemOp;

/// One lane's frame, extracted from the bit planes: `w^phase * prod_q S_q^{s_pow[q]} * X^x Z^z`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneFrame {
    /// Bit sets over system qubits, `ceil(n / 64)` words.
    pub x: Vec<u64>,
    pub z: Vec<u64>,
    /// `S` power per system qubit, in `0..4`.
    pub s_pow: Vec<u8>,
    /// In `Z/8`. Zero in the frame passed to `givens`.
    pub phase: u8,
}

/// One lane's tracker state.
pub trait LaneTracker: Send {
    /// A `Givens` between system qubits `p` and `p + 1` with the angle register's value
    /// `angle`, applied after `before` (the lane's frame since the previous hand-off).
    ///
    /// # Errors
    /// Anything the tracker cannot represent (reported as the lane's rejection reason).
    fn givens(&mut self, before: &LaneFrame, p: usize, angle: u64) -> Result<(), String>;

    /// A `Givens` between system modes `p < q` (not necessarily adjacent; spec/DESIGN.md section 15).
    /// The default accepts only `q = p + 1` and forwards to `givens`.
    ///
    /// # Errors
    /// As `givens`, or non-adjacent modes for a tracker that does not override this.
    fn givens_modes(
        &mut self,
        before: &LaneFrame,
        p: usize,
        q: usize,
        angle: u64,
    ) -> Result<(), String> {
        if q != p + 1 {
            return Err(format!("this tracker needs adjacent modes, got ({p}, {q})"));
        }
        self.givens(before, p, angle)
    }

    /// Final comparison: the lane's operator is `after * (tracked operator)`, which must equal
    /// `reference` exactly (phase included). Called only for control-1 lanes; a control-0 lane
    /// with a tracker is also passed here with `reference = None` and must be the identity.
    ///
    /// # Errors
    /// The operator differs from the reference (the message is the rejection reason).
    fn finish(&mut self, after: &LaneFrame, reference: Option<&SystemOp>) -> Result<(), String>;
}

/// Creates lane trackers and states the cost model of `Givens`.
pub trait TrackerFactory: Sync {
    fn lane(&self, system_qubits: usize) -> Box<dyn LaneTracker>;
    /// Toffolis charged per executed `Givens` (spec/DESIGN.md section 15).
    fn givens_toffoli_cost(&self) -> f64;
    /// Width of the phase-gradient register, added to `Q_peak` when any `Givens` is used.
    fn phase_gradient_qubits(&self) -> u64;
    /// The angle-register value of `pi / 2` (`2^(beta - 2)`), which `SpinSwap` applies
    /// (spec/SPEC-SA.md section 11). `None`: this tracker cannot run `SpinSwap`.
    fn quarter_turn(&self) -> Option<u64> {
        None
    }
    /// Tapered sos-sa specs only (spec/SPEC-SA.md section 13): the Toffolis charged for one
    /// executed `Givens` on system modes `(p, q)`, which depend on its chain position. `None` for
    /// every other tracker, whose charge is `givens_toffoli_cost` per `Givens`, unchanged.
    fn givens_charge(&self, p: usize, q: usize) -> Option<u64> {
        let _ = (p, q);
        None
    }
    /// The fermionic-Gaussian factory behind this tracker, if it is one. The fast engine
    /// (`crate::fastsim`) uses it to run its bit-identical kernels; for any other tracker it
    /// replays the trait's own `lane`, so the default `None` is always safe.
    fn as_gaussian(&self) -> Option<&super::gaussian::GaussianFactory> {
        None
    }
}
