//! Outcome-gated erasure of a comparison bit (measurement-based uncomputation, Gidney 2018,
//! arXiv:1709.06648, Fig. 3; the same move as ecdsa.fail's gated carry erasures).
//!
//! [`super::arith::unless_than`] erases `lt = [a < b]` by recomputing the `n - 1` lower carries
//! on every lane (`n - 1` Toffolis) and measuring them all out. [`unless_than_gated`] measures
//! `lt` first, in the X basis, with outcome `r`: that leaves the phase `(-1)^(r lt)`, which only
//! the lanes with `r = 1` need cancelled. Under `PushCondition(r)` it recomputes the lower carries,
//! applies `(-1)^lt` with Cliffords (`lt = MAJ(a_{n-1}, b_{n-1}, c_{n-1}) =
//! c ^ (a ^ c)(b ^ c)`, so `(-1)^lt = Z(c) CZ(a ^ c, b ^ c)`), and erases the carries by
//! measurement (their own phase fixups nested under the same condition). After `PopCondition`
//! every carry qubit is `|0>` on every lane (untouched where `r = 0`, measured where `r = 1`),
//! and an unconditional `R` at depth 0 ends its lifetime (`src/sim/liveness.rs` ends a qubit
//! only on a depth-0 `R` or `Hmr`).
//!
//! `r` is a fair coin on every lane (an X-basis outcome), so the expected cost is `(n - 1) / 2`
//! Toffolis instead of `n - 1`, with no extra qubits. The harness averages executed Toffolis
//! over lanes (spec/DESIGN.md section 8), so a walk that uses this has an expected-count ledger
//! (`sa_low::ledger`).
use super::arith::carries;
use super::lookup::hmr_keep;
use crate::circuit::{Builder, Qubit};

/// Erases `lt` (made by [`super::arith::less_than`] on the same, unchanged registers `a`, `bb`)
/// by an X-basis measurement and a phase correction gated on its outcome. Expected Toffolis
/// `(n - 1) / 2` (none on lanes whose outcome is 0), no qubits beyond [`super::arith::unless_than`]'s.
///
/// # Panics
/// If the widths differ or are zero.
pub fn unless_than_gated(b: &mut Builder, a: &[Qubit], bb: &[Qubit], lt: Qubit) {
    unless_than_gated_with(b, a, bb, lt, Gate::Exact);
}

/// Deliberate faults for the mutant tests (never used by a walk).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// The construction as documented.
    Exact,
    /// Drop the conditioned phase `(-1)^lt`.
    NoPhase,
    /// Correct on the lanes with outcome 0 instead of 1.
    FlipOutcome,
}

/// [`unless_than_gated`] with a possible fault (`Gate::Exact` is the construction).
#[doc(hidden)]
pub fn unless_than_gated_with(b: &mut Builder, a: &[Qubit], bb: &[Qubit], lt: Qubit, g: Gate) {
    let n = a.len();
    assert!(n > 0 && n == bb.len(), "bad comparator widths");
    let r = b.hmr(lt);
    if g == Gate::FlipOutcome {
        flip(b, r);
    }
    a.iter().for_each(|&q| b.x(q));
    b.push_condition(r);
    let cs = carries(b, a, bb, n - 1);
    if g != Gate::NoPhase {
        match cs.last() {
            None => b.cz(a[0], bb[0]),
            Some(&c) => {
                let (x, y) = (a[n - 1], bb[n - 1]);
                b.cx(c, x);
                b.cx(c, y);
                b.cz(x, y);
                b.cx(c, x);
                b.cx(c, y);
                b.z(c);
            }
        }
    }
    // Erase the lower carries by measurement, top first, under the same condition.
    for i in (0..cs.len()).rev() {
        let t = cs[i];
        let (x, y) = (a[i], bb[i]);
        match i.checked_sub(1).map(|j| cs[j]) {
            None => {
                let m = hmr_keep(b, t);
                b.cz_if(x, y, m);
            }
            Some(c) => {
                b.cx(c, t);
                b.cx(c, x);
                b.cx(c, y);
                let m = hmr_keep(b, t);
                b.cz_if(x, y, m);
                b.cx(c, x);
                b.cx(c, y);
            }
        }
    }
    b.pop_condition();
    if g == Gate::FlipOutcome {
        flip(b, r);
    }
    a.iter().for_each(|&q| b.x(q));
    for c in cs {
        b.free(c);
    }
}

fn flip(b: &mut Builder, r: crate::circuit::Bit) {
    let mut op = crate::circuit::Op::new(crate::circuit::OperationType::BitInvert);
    op.c_target = r.0;
    b.emit(op);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::common::arith::less_than;
    use crate::walk::shared::testsim::Sim;

    fn run(n: usize, g: Gate, seed: u64) -> Result<(u64, u64), String> {
        let mut b = Builder::new(0);
        b.declare_uniform(2 * n as u32);
        let a: Vec<Qubit> = (0..n as u32).map(|i| b.uniform(i)).collect();
        let bb: Vec<Qubit> = (n as u32..2 * n as u32).map(|i| b.uniform(i)).collect();
        let lt = less_than(&mut b, &a, &bb);
        let probe = b.ops().len();
        unless_than_gated_with(&mut b, &a, &bb, lt, g);
        let gated_ops = b.ops()[probe..].to_vec();
        let ops = b.ops().to_vec();
        let mut total = 0;
        let inputs: Vec<Qubit> = [a, bb].concat();
        for x in 0..1u64 << (2 * n) {
            let mut s = Sim::new(&b, seed ^ x.wrapping_mul(0x2545_F491));
            s.set_uniform(x);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                s.run(&ops);
                s.assert_clean();
            }));
            if r.is_err() {
                return Err(format!("dirty or phase garbage at x = {x}"));
            }
            total += s.toffolis;
            if s.read(&inputs) != x {
                return Err(format!("inputs changed at x = {x}"));
            }
        }
        let static_ccx = gated_ops
            .iter()
            .filter(|o| o.kind == crate::circuit::OperationType::CCX)
            .count() as u64;
        Ok((total, static_ccx))
    }

    /// Exhaustive at widths 1..=5 over several outcome seeds: clean, phase-free, inputs kept,
    /// and on average half the recompute's Toffolis.
    #[test]
    fn gated_erasure_is_exact() {
        for n in 1..=5 {
            for seed in 0..4u64 {
                let (tof, st) = run(n, Gate::Exact, seed * 7919 + 1).unwrap();
                assert_eq!(st as usize, n - 1, "static Toffolis of the gated block");
                let lanes = 1u64 << (2 * n);
                // n Toffolis for less_than on every lane, plus the gated recompute on about half.
                let gated = tof - lanes * n as u64;
                assert!(gated <= lanes * (n as u64 - 1));
                if n > 2 {
                    let frac = gated as f64 / (lanes * (n as u64 - 1)) as f64;
                    assert!((0.3..0.7).contains(&frac), "n {n}: gated fraction {frac}");
                }
            }
        }
    }

    /// Mutants: without the conditioned phase, or gated on the wrong outcome, some lane is left
    /// with phase garbage.
    #[test]
    fn gated_erasure_mutants_are_caught() {
        for n in 1..=4 {
            for g in [Gate::NoPhase, Gate::FlipOutcome] {
                let caught = (0..4u64).any(|seed| run(n, g, seed * 31 + 5).is_err());
                assert!(caught, "{g:?} at n = {n} must be caught");
            }
        }
    }
}
