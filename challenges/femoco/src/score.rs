//! Evaluation: validate a submission's three files against a spec, count, and score
//! (spec/DESIGN.md sections 6, 8, 9 and 11). `eval_circuit` is a thin wrapper
//! around `evaluate` plus the writers in `report`.
mod report;

pub use report::{append_results_row, sa_score_lambda, score_json, ResultsRow};

use crate::circuit::{OpsFile, SEG_INNER_BEGIN, SEG_INNER_END};
use crate::facts::{static_facts, CircuitFacts};
use crate::fiat_shamir::{sample, sample_nested, Digests};
use crate::lanemap::{self, LaneMap};
use crate::sim::{compile, compile_nested, compile_sa, validate, Inner, Layout, TrackerFactory};
use crate::spec::rounding::{Estimate, RoundingClass};
use crate::spec::{EncodingSpec, Exact};
use crate::taxonomy::{AxisStatus, AxisVerdict, Family};
use num_bigint::{BigInt, BigUint};
use sha2::{Digest, Sha256};
use std::time::Instant;

/// An engine by name (`eval_circuit --engine`); `Engine::lane_engine` gives what
/// [`evaluate_with`] takes.
pub use crate::fastsim::Engine;

/// Rejection threshold for the lane map's rounding error: 0.1 mHa, exactly `1/10000` Ha.
#[must_use]
pub fn max_rounding_error() -> Exact {
    Exact::new(BigInt::from(1), BigUint::from(10_000u32))
}

/// Energy target used only for the literature-convention total (Lee et al. 2021).
pub const EPSILON: f64 = 0.0016;
/// Confidence parameter of the sampling bound.
pub const DELTA: f64 = 1e-6;
/// Free Fiat-Shamir re-draws a submitter is assumed able to try, as a power of two. Any byte of
/// `family.out.json` or `ops.bin` acts as a nonce, so a submitter who knows which lanes are wrong
/// can search offline for a sample that misses them; with `T` tries the single-draw confidence
/// `1 - delta` falls to about `1 - T delta`. The grinding bounds below keep confidence `1 - delta`
/// against `T = 2^GRINDING_LOG2` tries, at the price of `ln T` in the numerator.
pub const GRINDING_LOG2: u32 = 40;

/// `ln(T / delta)` for `T = 2^draws_log2` sample draws.
fn log_term(draws_log2: u32) -> f64 {
    (1.0 / DELTA).ln() + f64::from(draws_log2) * std::f64::consts::LN_2
}
/// Default number of sampled lanes, `2^19`. Measured (tests/harness_timing.rs): a 30-million-op
/// synthetic circuit (108 system qubits, 20 uniform bits) evaluates in 46 s with 4 threads on an
/// Apple M4 Max, plus 3 s to read `ops.bin`; the lane simulation scales linearly in ops x lanes
/// and was insensitive to width (430 vs 4130 qubits). So about two minutes on four cores covers
/// roughly 75 million ops on that machine, less on slower cores. `f_bound = ln(1e6) / 2^19 =
/// 2.6e-5`.
pub const DEFAULT_SAMPLES: usize = 1 << 19;

/// `family.out.json`: the declared family plus the spec id the walk implements.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FamilyOut {
    #[serde(flatten)]
    pub family: Family,
    pub spec: String,
}

/// What `evaluate` needs. The spec is loaded by the caller from the trusted `specs/`.
pub struct Inputs<'a> {
    pub spec: &'a dyn EncodingSpec,
    pub lanemap: &'a [u8],
    pub family: &'a [u8],
    pub ops: &'a OpsFile,
    pub samples: usize,
    pub tracker: Option<&'a dyn TrackerFactory>,
    /// The taxonomy check (`taxonomy::check` with the loaded taxonomy in production).
    pub check: &'a dyn Fn(&Family, &CircuitFacts) -> Vec<AxisVerdict>,
}

/// A passing evaluation.
#[derive(Clone, Debug)]
pub struct Evaluation {
    pub family: FamilyOut,
    pub facts: CircuitFacts,
    pub verdicts: Vec<AxisVerdict>,
    pub lambda: Exact,
    /// The exact 1-norm rounding error `sum_t |c_t - c^_t|`, whatever the spec's rounding class.
    pub rounding_error: Exact,
    /// sos-sa specs that declare `sos-ground-cs-v1` only (spec/SPEC-SA.md section 12): the rule,
    /// its budget and the bound's parts. `rounding_error` is then the ground-energy bound.
    pub rounding_rule: Option<RoundingRule>,
    /// Specs of the estimated rounding class only (spec/SPEC-SA.md section 14): what Low et al.'s procedure
    /// found for this lane map. `None` for every rigorous run, whose rule is the 0.1 mHa above.
    pub rounding_estimate: Option<Estimate>,
    /// `C_step`: executed Toffolis per lane (+ Givens charge + reflection).
    pub toffoli: f64,
    pub reflection_toffoli: u64,
    /// Nested lane maps only: the charged inner `Reflect` Toffolis per step and the paired and
    /// diagonal sample counts (spec/DESIGN.md section 16).
    pub nested: Option<NestedStats>,
    pub qubits: u64,
    pub samples: usize,
    /// Sampled lanes with control bit 1: the only lanes that test the encoded operator
    /// (`implied_error_bound`).
    pub control_one_samples: usize,
    /// Executed `SpinSwap` ops per lane (sos-sa only; each charged `spin_swap_toffoli`).
    pub spin_swaps: f64,
    /// The Toffolis those `SpinSwap`s add to `C_step` per lane.
    pub spin_swap_toffoli: f64,
    /// Tapered sos-sa specs only (spec/SPEC-SA.md section 13): the per-lane Givens charge, each
    /// Givens at its chain position's width. `None` for every other spec.
    pub givens_toffoli_tapered: Option<f64>,
    pub cliffords: f64,
    pub measurements: f64,
    pub resets: f64,
    pub digests: Digests,
    pub seconds: f64,
}

/// A spec-declared rounding rule and what the lane map met it with (spec/SPEC-SA.md section 12).
#[derive(Clone, Debug)]
pub struct RoundingRule {
    pub rule: String,
    pub budget: Exact,
    pub bound: crate::lanemap::sa_nested::GroundBound,
    /// The 1-norm `sum_t |c_t - c^_t|` of the same lane map (reported; not the rule here).
    pub one_norm: Exact,
}

/// What a nested evaluation adds to `score.json`.
#[derive(Clone, Copy, Debug)]
pub struct NestedStats {
    pub inner_width: u32,
    pub inner_reflect_toffoli: f64,
    pub paired_samples: usize,
    pub diagonal_samples: usize,
    /// The paired and diagonal lanes with control bit 1 (`implied_error_bound`).
    pub paired_control_one: usize,
    pub diagonal_control_one: usize,
}

impl NestedStats {
    /// `ln(1/delta) / K_p` and `ln(1/delta) / K_d`.
    #[must_use]
    pub fn f_bounds(&self) -> (f64, f64) {
        self.f_bounds_at(0)
    }
    /// The same against `2^draws_log2` free re-draws of the seed.
    #[must_use]
    pub fn f_bounds_at(&self, draws_log2: u32) -> (f64, f64) {
        let f = |k: usize| log_term(draws_log2) / k.max(1) as f64;
        (f(self.paired_samples), f(self.diagonal_samples))
    }
    /// The same over the control-1 paired and diagonal lanes only.
    #[must_use]
    pub fn f_bounds_control_one_at(&self, draws_log2: u32) -> (f64, f64) {
        let f = |k: usize| log_term(draws_log2) / k.max(1) as f64;
        (f(self.paired_control_one), f(self.diagonal_control_one))
    }
}

impl Evaluation {
    /// `lambda x C_step x Q_peak`. For an `sos-sa` spec the lambda is the spectrum-amplified walk's
    /// `lambda_eff` (spec/SPEC-SA.md section 6): the larger of the certificate's bound and the
    /// paper's, or the worst-case `Lambda` when the certificate gives none for this run.
    #[must_use]
    pub fn score(&self) -> f64 {
        self.score_lambda() * self.toffoli * self.qubits as f64
    }
    /// The lambda `score` uses: `lambda_decl`, except for `sos-sa` specs (see `score`).
    #[must_use]
    pub fn score_lambda(&self) -> f64 {
        if self.facts.encoding == crate::spec::sa::ENCODING {
            if let Some(l) = sa_score_lambda(self) {
                return l;
            }
        }
        self.lambda.to_f64()
    }
    /// `ceil(pi lambda / (2 epsilon)) * C_step`.
    #[must_use]
    pub fn total_toffoli_lit(&self) -> f64 {
        (std::f64::consts::PI * self.lambda.to_f64() / (2.0 * EPSILON)).ceil() * self.toffoli
    }
    /// `ln(1 / delta) / K`: a single draw of the sample.
    #[must_use]
    pub fn f_bound(&self) -> f64 {
        self.f_bound_at(0)
    }
    /// `ln(T / delta) / K` against `T = 2^draws_log2` free re-draws of the seed.
    #[must_use]
    pub fn f_bound_at(&self, draws_log2: u32) -> f64 {
        log_term(draws_log2) / self.samples as f64
    }
    /// The encoded operator's distance from the declared one implied by the sampling: flat
    /// `2 lambda f_1`, nested `2 lambda (2 f_p1 + f_d1)` (spec/DESIGN.md section 16), where
    /// each `f` is `ln(1/delta) / K` over the sampled lanes with control bit 1 only.
    ///
    /// Only control-1 lanes test the encoded operator, and about half the lanes have control 0.
    /// The formula first used `f_bound` over all lanes, which understated the bound by about 2:
    /// a circuit wrong on every control-1 lane of a fraction `g` of uniform values has only
    /// `g / 2` of its lanes wrong but is `2 lambda g` from the declared operator.
    #[must_use]
    pub fn implied_error_bound(&self) -> f64 {
        self.implied_error_bound_at(0)
    }
    /// [`Self::implied_error_bound`] against `2^draws_log2` free re-draws of the seed.
    #[must_use]
    pub fn implied_error_bound_at(&self, draws_log2: u32) -> f64 {
        let lambda = self.lambda.to_f64();
        match &self.nested {
            None => 2.0 * lambda * log_term(draws_log2) / self.control_one_samples.max(1) as f64,
            Some(n) => {
                let (fp, fd) = n.f_bounds_control_one_at(draws_log2);
                2.0 * lambda * (2.0 * fp + fd)
            }
        }
    }
}

fn sha(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

/// Validates, counts and scores one submission. Every rejection names its reason.
///
/// # Errors
/// The first reason the submission is rejected.
pub fn evaluate(inp: &Inputs<'_>) -> Result<Evaluation, String> {
    evaluate_with(inp, validate::reference, None)
}

/// What one engine run showed besides the `Evaluation` (for the equivalence harness,
/// `crate::equiv`): every sampled lane's verdict (`validate::VERDICT_PASS`, a category index,
/// or `validate::VERDICT_UNCHECKED`), the lanes handed to the engine and its whole `Outcome`.
/// Left empty when the run stopped before the engine (a static rejection).
#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub verdicts: Vec<u8>,
    pub lanes: Vec<crate::fiat_shamir::Lane>,
    /// `format!("{:?}", outcome)`: tallies, failure counts and first failures, messages included.
    pub outcome: String,
    pub ran: bool,
}

/// [`evaluate`] with the lane engine `engine` in place of the reference
/// (`validate::reference`); everything before and after the engine is the same code. With
/// `probe`, the engine also records each lane's verdict (spec/FAST-EVALUATOR.md section 5).
///
/// # Errors
/// As [`evaluate`].
pub fn evaluate_with(
    inp: &Inputs<'_>,
    engine: validate::Engine,
    mut probe: Option<&mut Probe>,
) -> Result<Evaluation, String> {
    let mut run_engine = |ctx: &validate::Context<'_>,
                          nested: Option<&validate::Nested<'_>>,
                          lanes: &[crate::fiat_shamir::Lane]| {
        let Some(p) = probe.as_deref_mut() else {
            return engine(ctx, nested, lanes, None);
        };
        p.verdicts = vec![validate::VERDICT_UNCHECKED; lanes.len()];
        let out = engine(ctx, nested, lanes, Some(&mut p.verdicts));
        p.lanes = lanes.to_vec();
        p.outcome = format!("{out:?}");
        p.ran = true;
        out
    };
    let t0 = Instant::now();
    let fam: FamilyOut =
        serde_json::from_slice(inp.family).map_err(|e| format!("family.out.json: {e}"))?;
    if fam.spec != inp.spec.id() {
        return Err(format!(
            "family.out.json names spec {} but {} was loaded",
            fam.spec,
            inp.spec.id()
        ));
    }
    let lm = lanemap::parse(inp.lanemap, inp.spec)?;
    let (rounding_error, rounding_rule, rounding_estimate) = check_lanemap(lm.as_ref(), inp.spec)?;
    let layout = Layout {
        system: inp.spec.system_qubits(),
        uniform: lm.uniform_bits(),
    };
    let inner = lm.inner_bits().map(|(lo, width)| Inner { lo, width });
    let compiled = match inner {
        // SpinSwap (spec/SPEC-SA.md section 11) exists for sos-sa specs only.
        Some(_) if inp.spec.encoding() == crate::spec::sa::ENCODING => {
            compile_sa(&inp.ops.ops, &layout, inp.tracker, inner)?
        }
        None => compile(&inp.ops.ops, &layout, inp.tracker)?,
        Some(_) => compile_nested(&inp.ops.ops, &layout, inp.tracker, inner)?,
    };
    let digests = Digests {
        spec: inp.spec.payload_sha256(),
        lanemap: sha(inp.lanemap),
        family: sha(inp.family),
        ops: inp.ops.sha256,
    };
    let k = inp.samples.max(1);
    let reference = |s: u64| lm.reference_op(inp.spec, s);
    let reference_nested = |s: u64, after: u64| lm.reference_nested(inp.spec, s, after);
    let uniform_after = |s: u64| lm.uniform_after(s);
    let control_one;
    let (out, nested) = match inner {
        None => {
            let smp = sample(&digests, layout.uniform, k);
            control_one = smp.lanes.iter().filter(|l| l.c).count();
            let ctx = validate::Context {
                compiled: &compiled,
                layout,
                hmr_key: smp.hmr_key,
                reference: &reference,
                uniform_after: &uniform_after,
                tracker: inp.tracker,
            };
            (run_engine(&ctx, None, &smp.lanes), None)
        }
        Some(inner) => {
            let smp = sample_nested(&digests, layout.uniform, inner.lo, inner.width, k);
            let ctx = validate::Context {
                compiled: &compiled,
                layout,
                hmr_key: smp.sample.hmr_key,
                reference: &reference,
                uniform_after: &uniform_after,
                tracker: inp.tracker,
            };
            let n = validate::Nested {
                lo: inner.lo,
                width: inner.width,
                after: &smp.after,
                reference: &reference_nested,
            };
            let diagonal = smp.diagonal.iter().filter(|&&d| d).count();
            let ones = |diag: bool| {
                smp.sample
                    .lanes
                    .iter()
                    .zip(&smp.diagonal)
                    .filter(|(l, &d)| l.c && d == diag)
                    .count()
            };
            control_one = ones(false) + ones(true);
            let stats = NestedStats {
                inner_width: inner.width,
                inner_reflect_toffoli: 0.0,
                paired_samples: k - diagonal,
                diagonal_samples: diagonal,
                paired_control_one: ones(false),
                diagonal_control_one: ones(true),
            };
            (run_engine(&ctx, Some(&n), &smp.sample.lanes), Some(stats))
        }
    };
    if let Some(why) = out.rejection() {
        return Err(why);
    }
    let per = |v: u64| v as f64 / k as f64;
    let mut facts = static_facts(&inp.ops.ops, &layout);
    fill_facts(&mut facts, inp.spec, lm.as_ref(), &compiled, &out.tally, k);
    // compile_nested accepted the nested structure (spec/DESIGN.md section 16.3).
    facts.nested_validated = inner.is_some();
    // Inner-block hints mean something only for a nested lane map. Elsewhere the ops between
    // them are counted under codes 3/4 and would hide from SELECT's counts (code 1), which the
    // `select` signatures read (a unary-iteration SELECT wrapped in 3/4 verified
    // `clifford-mask`).
    let inner_hints = [SEG_INNER_BEGIN, SEG_INNER_END]
        .iter()
        .any(|&c| facts.segment_op_counts.contains_key(&(c as u8)));
    if inner.is_none() && inner_hints {
        facts.segments_validated = false;
    }
    facts.inner_uniform_bits = inner.map(|i| i.width);
    let verdicts = (inp.check)(&fam.family, &facts);
    if let Some(v) = verdicts
        .iter()
        .find(|v| matches!(v.status, AxisStatus::Contradicted(_)))
    {
        let AxisStatus::Contradicted(why) = &v.status else {
            unreachable!()
        };
        return Err(format!(
            "taxonomy: declared {} = {} is contradicted by the circuit: {why}",
            v.axis, v.value
        ));
    }
    let givens_cost = inp.tracker.map_or(0.0, |t| t.givens_toffoli_cost());
    // Tapered sos-sa specs (spec/SPEC-SA.md section 13): each executed Givens is charged at its
    // chain position's width, summed per lane by the simulator. Every other spec: the uniform
    // charge, computed exactly as before.
    let tapered = inp.tracker.is_some_and(|t| t.givens_charge(0, 1).is_some());
    let givens_toffoli = if tapered {
        per(out.tally.givens_charge)
    } else {
        givens_cost * per(out.tally.givens)
    };
    let reflection_toffoli = u64::from(layout.uniform.saturating_sub(2));
    // Nested: each executed inner Reflect on w qubits is charged max(w - 2, 0) Toffolis
    // (spec/DESIGN.md section 16.1).
    let nested = nested.map(|n| NestedStats {
        inner_reflect_toffoli: f64::from(n.inner_width.saturating_sub(2)) * per(out.tally.reflects),
        ..n
    });
    let spin_swaps = per(out.tally.spin_swaps);
    let spin_swap_charge = spin_swap_toffoli(layout.system) * spin_swaps;
    let toffoli = per(out.tally.ccx + out.tally.ccz)
        + givens_toffoli
        + spin_swap_charge
        + reflection_toffoli as f64
        + nested.map_or(0.0, |n| n.inner_reflect_toffoli);
    let gradient = if compiled.uses_givens {
        inp.tracker.map_or(0, |t| t.phase_gradient_qubits())
    } else {
        0
    };
    Ok(Evaluation {
        family: fam,
        verdicts,
        lambda: lm.lambda_decl(),
        rounding_error,
        rounding_rule,
        rounding_estimate,
        toffoli,
        reflection_toffoli,
        nested,
        qubits: compiled.q_peak + gradient,
        samples: k,
        control_one_samples: control_one,
        spin_swaps,
        givens_toffoli_tapered: tapered.then_some(givens_toffoli),
        spin_swap_toffoli: spin_swap_charge,
        cliffords: per(out.tally.cliffords),
        measurements: per(out.tally.hmr),
        resets: per(out.tally.resets),
        digests,
        seconds: t0.elapsed().as_secs_f64(),
        facts,
    })
}

/// Toffolis charged per executed `SpinSwap` on `2N` system qubits: `N + 1` (spec/SPEC-SA.md
/// section 11). In the spin-blocked Jordan-Wigner order the fermionic swap of the two spin
/// blocks is the qubit swap of the blocks times `(-1)^(N_up N_down)`; controlled, that is `N`
/// controlled swaps (one Toffoli each) and one `CCZ` on the two block parities (computed in
/// place by CNOT ladders). The layer `F = prod_p G_{2p, 2p+1}(pi / 2)` it applies is that swap
/// times a Clifford (a `Z` on one mode of each pair), controlled by `CZ`s.
#[must_use]
pub fn spin_swap_toffoli(system_qubits: usize) -> f64 {
    (system_qubits / 2 + 1) as f64
}

/// Proves the declared lane map realizes the spec's coefficients to within 0.1 mHa, exactly: the
/// rule for every spec of the rigorous class without a `rounding` block. Two spec-scoped
/// alternatives, never both on one spec:
/// - an sos-sa spec that declares `sos-ground-cs-v1` (spec/SPEC-SA.md section 12): the rounded
///   operator's ground energy must be within the spec's budget (rigorous);
/// - the pinned estimated-class specs (spec/SPEC-SA.md section 14): the same exact 1-norm is computed and
///   reported, and acceptance is Low et al.'s estimated procedure (`LaneMap::rounding_estimate`).
///
/// The rule and class come from the trusted spec, never from the lane map.
fn check_lanemap(
    lm: &dyn LaneMap,
    spec: &dyn EncodingSpec,
) -> Result<(Exact, Option<RoundingRule>, Option<Estimate>), String> {
    if lm.uniform_bits() > 63 {
        return Err(format!("lane map: u = {} above 63", lm.uniform_bits()));
    }
    let sa = spec.as_any().downcast_ref::<crate::spec::sa::SaSpec>();
    if let Some((d, crate::spec::sa::Rounding::Ground { budget, e_up })) =
        sa.map(|d| (d, &d.rounding))
    {
        let g = e_up.sub(&d.e_sos);
        let gb = lm.ground_bound(spec, &g).ok_or_else(|| {
            format!(
                "spec {} declares rounding rule {}, which lane map {} does not support",
                spec.id(),
                crate::spec::sa::GROUND_RULE,
                lm.family()
            )
        })??;
        if !gb.within(budget) {
            return Err(format!(
                "lane map ground-energy rounding bound {:.6e} Ha exceeds the spec's budget {:.6e} \
                 Ha (rule {}, exact bound {}, budget {budget})",
                gb.bound.to_f64(),
                budget.to_f64(),
                crate::spec::sa::GROUND_RULE,
                gb.bound
            ));
        }
        let one_norm = lm.rounding_error(spec)?;
        let bound = gb.bound.clone();
        return Ok((
            bound,
            Some(RoundingRule {
                rule: crate::spec::sa::GROUND_RULE.to_string(),
                budget: budget.clone(),
                bound: gb,
                one_norm,
            }),
            None,
        ));
    }
    let err = lm.rounding_error(spec)?;
    match spec.rounding_class() {
        RoundingClass::Rigorous => {
            if err > max_rounding_error() {
                return Err(format!(
                    "lane map rounding error {:.6e} Ha exceeds 0.1 mHa (exact {err})",
                    err.to_f64()
                ));
            }
            Ok((err, None, None))
        }
        RoundingClass::EstimatedLow2025(params) => {
            let est = lm.rounding_estimate(spec, &params)?;
            if let Some(why) = est.rejection() {
                return Err(why);
            }
            Ok((err, None, Some(est)))
        }
    }
}

fn fill_facts(
    f: &mut CircuitFacts,
    spec: &dyn EncodingSpec,
    lm: &dyn LaneMap,
    c: &compile::Compiled,
    t: &crate::sim::lanes::Tally,
    k: usize,
) {
    let per = |v: u64| v as f64 / k as f64;
    f.spec_id = spec.id().to_string();
    f.encoding = spec.encoding().to_string();
    f.lane_map_family = lm.family().to_string();
    f.u = lm.uniform_bits();
    (f.k, f.mu) = lm
        .alias_bits()
        .map_or((None, None), |(k, mu)| (Some(k), Some(mu)));
    f.executed_ccx = per(t.ccx);
    f.executed_ccz = per(t.ccz);
    f.executed_hmr = per(t.hmr);
    f.executed_givens = per(t.givens);
    f.segment_ancilla_peak = c.segment_ancilla_peak.clone();
    f.q_peak = c.q_peak;
}
