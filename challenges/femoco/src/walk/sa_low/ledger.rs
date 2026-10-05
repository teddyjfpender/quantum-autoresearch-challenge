//! A per-stage count of what the walk emits: `CCX`/`CCZ` (Toffolis) and `Givens` ops, read off
//! the builder's op stream between marks. It changes nothing in the stream.
//!
//! Without `sa-toff`'s gated comparator erasure (lever `g`), every Toffoli this walk emits is
//! unconditioned (no `CCX` inside a condition block), so the static count of a stage is also its
//! executed count on every lane ("static == harness"); the harness's `C_step` is the sum of
//! these, plus `2 (beta - 2)` per `Givens` and the two reflections (`tests` checks the sum
//! against `score::evaluate`).
//!
//! **Expected counts.** A gated erasure (`walk::common::arith_gated`) puts Toffolis under
//! `PushCondition(r)` with `r` an X-basis outcome, a fair coin on every lane. The harness averages
//! executed Toffolis over its sampled lanes (spec/DESIGN.md section 8), so its `C_step` is then an
//! estimate of the expected count, not an identity. [`Ledger::expected`] weights each Toffoli by
//! `2^-d`, `d` the number of condition bits it sits under (the enclosing `PushCondition`s plus its
//! own `c_condition`), which is its execution probability when every condition bit is an
//! independent fair measurement outcome (true of every conditioned Toffoli this walk emits). The
//! static column [`Ledger::rows`] still counts every emitted Toffoli once.
use crate::circuit::{Builder, OperationType as K, NONE};

#[derive(Clone, Debug, Default)]
pub struct Ledger {
    /// `(stage, Toffolis, Givens)`, in emission order. Stages inside the inner copy are listed
    /// once; the copy runs twice.
    pub rows: Vec<(String, u64, u64)>,
    /// Expected executed Toffolis of each row (same order as `rows`; module docs).
    pub expected: Vec<f64>,
    /// End of each row in the op stream (index one past its last op; same order as `rows`), for
    /// attributing live qubits to the stage that allocated them (`tests_combo::anatomy`).
    pub ends: Vec<usize>,
    mark: usize,
}

impl Ledger {
    /// Starts counting from the current end of the stream.
    pub fn start(&mut self, b: &Builder) {
        self.mark = b.ops().len();
    }

    /// Records everything since the last mark as `stage`.
    pub fn stage(&mut self, b: &Builder, stage: &str) {
        let ops = &b.ops()[self.mark..];
        let tof = ops
            .iter()
            .filter(|o| matches!(o.kind, K::CCX | K::CCZ))
            .count() as u64;
        let giv = ops.iter().filter(|o| o.kind == K::Givens).count() as u64;
        let mut depth = 0i32;
        let mut exp = 0.0;
        for o in ops {
            match o.kind {
                K::PushCondition => depth += 1,
                K::PopCondition => depth -= 1,
                K::CCX | K::CCZ => {
                    let d = depth + i32::from(o.c_condition != NONE);
                    exp += 0.5f64.powi(d);
                }
                _ => {}
            }
        }
        self.rows.push((stage.to_string(), tof, giv));
        self.expected.push(exp);
        self.ends.push(b.ops().len());
        self.mark = b.ops().len();
    }

    /// Toffolis of every row whose stage starts with `prefix`.
    #[must_use]
    pub fn sum(&self, prefix: &str) -> (u64, u64) {
        self.rows
            .iter()
            .filter(|r| r.0.starts_with(prefix))
            .fold((0, 0), |(t, g), r| (t + r.1, g + r.2))
    }

    /// Expected Toffolis and the Givens of every row whose stage starts with `prefix`.
    #[must_use]
    pub fn expected_sum(&self, prefix: &str) -> (f64, u64) {
        self.rows
            .iter()
            .zip(&self.expected)
            .filter(|(r, _)| r.0.starts_with(prefix))
            .fold((0.0, 0), |(t, g), (r, e)| (t + e, g + r.2))
    }
}
