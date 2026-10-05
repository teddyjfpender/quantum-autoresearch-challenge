//! `score.json` and `results.tsv` (spec/DESIGN.md section 11).
use super::{Evaluation, EPSILON};
use crate::taxonomy::AxisStatus;
use serde_json::json;
use std::io::Write;
use std::path::Path;

pub const RESULTS_HEADER: &str =
    "unix_time\tcommit\tspec\tfamily_name\tfamily_axes\tlambda\ttoffoli\t\
qubits\tscore\ttotal_toffoli_lit\tsamples\tstatus\tnote";

fn axes_named(ev: &Evaluation, verified: bool) -> Vec<String> {
    ev.verdicts
        .iter()
        .filter(|v| (v.status == AxisStatus::Verified) == verified)
        .map(|v| v.axis.clone())
        .collect()
}

/// The canonical `score.json` body for a passing run.
#[must_use]
pub fn score_json(ev: &Evaluation) -> serde_json::Value {
    let f_bound = ev.f_bound();
    let lambda = ev.lambda.to_f64();
    let fam = &ev.family.family;
    let mut out = json!({
        "score": ev.score(),
        "metrics": {
            "toffoli": ev.toffoli,
            "qubits": ev.qubits,
            "lambda": lambda,
            "lambda_exact": ev.lambda.to_string(),
            "spec": ev.facts.spec_id,
            "instance": ev.facts.spec_id.split('-').next().unwrap_or(""),
            "encoding": ev.facts.encoding,
            "lane_map": ev.facts.lane_map_family,
            "total_toffoli_lit": ev.total_toffoli_lit(),
            "epsilon": EPSILON,
            "rounding_error": ev.rounding_error.to_f64(),
            "rounding_error_exact": ev.rounding_error.to_string(),
            "samples": ev.samples,
            "control_one_samples": ev.control_one_samples,
            "f_bound": f_bound,
            "implied_error_bound": ev.implied_error_bound(),
            "grinding_draws_log2": crate::score::GRINDING_LOG2,
            "f_bound_grinding": ev.f_bound_at(crate::score::GRINDING_LOG2),
            "implied_error_bound_grinding": ev.implied_error_bound_at(crate::score::GRINDING_LOG2),
            "reflection_toffoli": ev.reflection_toffoli,
            "executed": {
                "ccx": ev.facts.executed_ccx,
                "ccz": ev.facts.executed_ccz,
                "hmr": ev.facts.executed_hmr,
                "givens": ev.facts.executed_givens,
            },
            "cliffords": ev.cliffords,
            "measurements": ev.measurements,
            "resets": ev.resets,
            "depth": ev.facts.depth,
            "toffoli_depth": ev.facts.toffoli_depth,
            "family": {
                "name": fam.name,
                "parent": fam.parent,
                "taxonomy_version": fam.taxonomy_version,
                "axes": fam.axes,
            },
            "verified_axes": axes_named(ev, true),
            "declared_axes": axes_named(ev, false),
            "digests": {
                "ops": hex::encode(ev.digests.ops),
                "lanemap": hex::encode(ev.digests.lanemap),
                "family": hex::encode(ev.digests.family),
                "spec": hex::encode(ev.digests.spec),
            },
            "eval_seconds": ev.seconds,
            // Report-only (spec/CONVENTIONS.md): the score above is unchanged.
            "conventions": conventions::report(&conventions::Run {
                spec: &ev.facts.spec_id,
                lane_map: &ev.facts.lane_map_family,
                lambda: ev.lambda.to_f64(),
                toffoli: ev.toffoli,
                qubits: ev.qubits,
                rounding_error: ev.rounding_error.to_f64(),
            }),
        }
    });
    // sos-sa runs only (spec/SPEC-SA.md section 6): the spectrum-amplified scoring view.
    if ev.facts.encoding == crate::spec::sa::ENCODING {
        out["metrics"]["spectral_amplification"] = conventions::sa_view(
            &conventions::Run {
                spec: &ev.facts.spec_id,
                lane_map: &ev.facts.lane_map_family,
                lambda: ev.lambda.to_f64(),
                toffoli: ev.toffoli,
                qubits: ev.qubits,
                rounding_error: ev.rounding_error.to_f64(),
            },
            ev.facts.executed_givens,
        );
        // SpinSwap (spec/SPEC-SA.md section 11): reported only where it was executed.
        if ev.spin_swaps > 0.0 {
            out["metrics"]["executed"]["spin_swap"] = json!(ev.spin_swaps);
            out["metrics"]["spin_swap_toffoli"] = json!(ev.spin_swap_toffoli);
        }
    }
    // Tapered sos-sa specs only (spec/SPEC-SA.md section 13).
    if let Some(g) = ev.givens_toffoli_tapered {
        out["metrics"]["rotation_widths"] = json!({
            "doc": "spec/SPEC-SA.md section 13",
            "givens_toffoli_per_step": g,
            "charge": "each executed Givens at chain position j costs 2 (w_j - 2) Toffolis (w_j the spec's width \
                       there); every other Givens 2 (beta - 2)",
        });
    }
    // Specs that declare a rounding rule only (spec/SPEC-SA.md section 12).
    if let Some(r) = &ev.rounding_rule {
        out["metrics"]["rounding_rule"] = json!({
            "rule": r.rule,
            "class": "rigorous: bound on the ground-energy error of the rounded sum of squares",
            "doc": "spec/SPEC-SA.md section 12",
            "bound_Ha": r.bound.bound.to_f64(),
            "bound_exact": r.bound.bound.to_string(),
            "budget_Ha": r.budget.to_f64(),
            "budget_exact": r.budget.to_string(),
            "within_0_1_mHa": r.bound.within(&crate::score::max_rounding_error()),
            "formula": "|E0(H~) - E0(H_spec)| <= 2 sqrt(G D_A) + D_A + L_B",
            "G_Ha": r.bound.g.to_f64(),
            "D_A": r.bound.d_a.to_f64(),
            "L_B_Ha": r.bound.l_b.to_f64(),
            "generators_linear": r.bound.linear,
            "generators": r.bound.generators,
            "one_norm_Ha": r.one_norm.to_f64(),
            "one_norm_within_0_1_mHa": r.one_norm <= crate::score::max_rounding_error(),
        });
    }
    // Estimated rounding class only (spec/SPEC-SA.md section 14): every rigorous run's score.json is unchanged.
    if let Some(est) = &ev.rounding_estimate {
        out["metrics"]["rounding_class"] = estimated_block(est, ev.rounding_error.to_f64());
    }
    // Nested runs only, so flat runs' score.json is unchanged (spec/DESIGN.md section 16).
    if let Some(n) = &ev.nested {
        let (fp, fd) = n.f_bounds();
        out["metrics"]["nested"] = json!({
            "inner_uniform_bits": n.inner_width,
            "inner_reflect_toffoli": n.inner_reflect_toffoli,
            "paired_samples": n.paired_samples,
            "diagonal_samples": n.diagonal_samples,
            "paired_control_one_samples": n.paired_control_one,
            "diagonal_control_one_samples": n.diagonal_control_one,
            "f_paired": fp,
            "f_diagonal": fd,
            "implied_error_bound_formula": "2 lambda (2 f_paired + f_diagonal), each f over the control-1 lanes, confidence 1 - 2 delta",
        });
    }
    out
}

/// `metrics.rounding_class` of a run under the estimated class (spec/SPEC-SA.md section 14).
fn estimated_block(est: &crate::spec::rounding::Estimate, one_norm: f64) -> serde_json::Value {
    use crate::spec::rounding::{ESTIMATED_CLASS, ESTIMATED_LABEL};
    let p = &est.params;
    let mut out = json!({
        "class": ESTIMATED_CLASS,
        "label": ESTIMATED_LABEL,
        "kind": "estimate, not a bound",
        "doc": "spec/SPEC-SA.md section 14",
        "rule": "Low et al. 2025 App. D and their cost script: sigma(b) = 2^(const - b) mHa (fitted to the CCSD(T) correlation-energy change under unbiased randomized rounding), b = ceil(log2(1 / split) + const), split = budget / sqrt(2), for the coefficient bits and the rotation bits",
        "rigorous_one_norm_Ha": one_norm,
        "rigorous_rule_met": one_norm <= 1e-4,
        "rigorous_rule": "the exact 1-norm rounding error at most 0.1 mHa, the rule of every rigorous spec; reported, not applied, under this class",
        "structural_check": "exact: lambda_decl = Lambda, and every count of every alias table is the floor or the ceiling of its ideal (the support of their randomized rounding)",
        "tables_checked": est.tables,
        "outer_bits_k_mu": [est.outer_bits.0, est.outer_bits.1],
        "inner_bits_k_mu": [est.inner_bits.0, est.inner_bits.1],
        "b_equiv_outer": est.b_equiv_outer,
        "b_equiv_inner": est.b_equiv_inner,
        "b_equiv_rule": "b_equiv = 1 + max{j : L 2^j <= 2^(k + mu)} for L items: their M = L 2^(b - 1) bins are no finer than the table's lanes; their fit covers the inner tables, this class applies it to the outer table too",
        "b_coeff": est.b_coeff(),
        "b_coeff_required": p.required_bits(p.coeff_const()),
        "beta": est.beta,
        "beta_required": p.required_bits(p.rot_const()),
        "sigma_coeff_mHa": est.sigma_coeff_mha(),
        "sigma_rot_mHa": est.sigma_rot_mha(),
        "sigma_trunc_mHa": est.sigma_trunc_mha(),
        "budget_mHa": p.budget_mha,
        "split_mHa": p.split_mha(),
        "constants": {
            "coeff_const_fits": p.coeff_fits,
            "rot_const_fits": p.rot_fits,
            "coeff_const": p.coeff_const(),
            "rot_const": p.rot_const(),
            "source": "Zenodo 17066718 get_walk_counts_different_bits_adjustments_for_paper_revision.py lines 112-143 (md5 41c45e6ce7c1c1df910d352d040daa1f)",
        },
        "accepted": est.accepted(),
    });
    // Eq. (D1) variants only (`spec::rounding::EstRule::EqD1`); the even-split specs' block is
    // unchanged.
    if let crate::spec::rounding::EstRule::EqD1 {
        sigma_pea_mha,
        eps_th_mha,
        total_mha,
    } = p.rule
    {
        out["rule"] = json!("Low et al. 2025 App. D Eq. (D1) as a whole: sqrt(sigma_PEA^2 + sigma_coeff(b_coeff)^2 + sigma_rot(beta)^2) + eps_th <= total, with sigma(b) = 2^(const - b) mHa (their fit); the split between coefficient and rotation bits is free (their script's even split is one choice of it)");
        out["eq_d1"] = json!({
            "name": crate::spec::rounding::EQ_D1_RULE,
            "sigma_pea_mHa": sigma_pea_mha,
            "eps_th_mHa": eps_th_mha,
            "total_mHa": total_mha,
            "lhs_mHa": est.d1_total_mha(),
        });
    }
    out
}

/// `lambda_eff_used` of an `sos-sa` evaluation (spec/SPEC-SA.md section 6), or `None`.
#[must_use]
pub fn sa_score_lambda(ev: &Evaluation) -> Option<f64> {
    conventions::sa_lambda_eff(&conventions::Run {
        spec: &ev.facts.spec_id,
        lane_map: &ev.facts.lane_map_family,
        lambda: ev.lambda.to_f64(),
        toffoli: ev.toffoli,
        qubits: ev.qubits,
        rounding_error: ev.rounding_error.to_f64(),
    })
    .map(|(_, _, used)| used)
}

/// The accuracy and qubit conventions for frontier claims (spec/CONVENTIONS.md), reported beside
/// the controlled cost comparison and never replacing it. Nothing here changes the score.
mod conventions {
    use serde_json::{json, Value};
    use std::f64::consts::PI;

    /// Energy target of the controlled cost comparison: the whole 1.6 mHa to phase estimation.
    pub const EPS_CONTROLLED: f64 = 0.0016;
    /// Lee et al. 2021's split (their Eqs. (23), (45)): 1.6 mHa total, 1.0 mHa to phase estimation,
    /// the remaining 0.6 mHa reserved for the approximation of the Hamiltonian.
    pub const EPS_TOTAL: f64 = 0.0016;
    pub const EPS_PEA: f64 = 0.001;
    pub const APPROX_BUDGET: f64 = 0.0006;
    pub const VERSION: &str = "femoco-conventions-v1";

    /// The pinned ground-energy certificates (specs/<id>/certificate.json), compiled in.
    const CERTIFICATES: [(&str, &str); 4] = [
        (
            "reiher-sa-v1",
            include_str!("../../specs/reiher-sa-v1/certificate.json"),
        ),
        (
            "li-sa-v1",
            include_str!("../../specs/li-sa-v1/certificate.json"),
        ),
        // The estimated-class specs state the same operator (the same sa.bin); their certificates
        // are the rigorous specs' with the spec id and spec.json hash rebound (spec/SPEC-SA.md).
        (
            "reiher-sa-est-v1",
            include_str!("../../specs/reiher-sa-est-v1/certificate.json"),
        ),
        (
            "li-sa-est-v1",
            include_str!("../../specs/li-sa-est-v1/certificate.json"),
        ),
    ];

    /// What the conventions need from an evaluation.
    pub struct Run<'a> {
        pub spec: &'a str,
        pub lane_map: &'a str,
        pub lambda: f64,
        pub toffoli: f64,
        pub qubits: u64,
        pub rounding_error: f64,
    }

    fn certificate(spec: &str) -> Option<Value> {
        CERTIFICATES
            .iter()
            .find(|(id, _)| *id == spec)
            .and_then(|(_, text)| serde_json::from_str(text).ok())
    }

    /// Walk steps `ceil(pi lambda / (2 eps))`.
    #[must_use]
    pub fn steps(lambda: f64, eps: f64) -> u64 {
        (PI * lambda / (2.0 * eps)).ceil() as u64
    }

    /// `ceil(log2(x))` for `x >= 1`.
    fn ceil_log2(x: u64) -> u64 {
        u64::from(64 - x.saturating_sub(1).leading_zeros())
    }

    /// Lee et al. 2021's phase-estimation qubits for `I` steps: `ceil(log(I + 1))` for the control
    /// register plus `ceil(log(I + 1)) - 1` for its unary iteration (their App. A item 1, Eq. (46)).
    #[must_use]
    pub fn phase_register(steps: u64) -> u64 {
        2 * ceil_log2(steps + 1) - 1
    }

    fn view(label: &str, lambda: f64, eps: f64, run: &Run<'_>) -> Value {
        let i = steps(lambda, eps);
        let reg = phase_register(i);
        json!({
            "label": label,
            "epsilon_pea": eps,
            "lambda": lambda,
            "steps": i,
            "total_toffoli": i as f64 * run.toffoli,
            "phase_register_qubits": reg,
            "qubits_lee": run.qubits + reg,
        })
    }

    fn estimate(cert: Option<&Value>) -> Value {
        let Some(e) = cert.map(|c| &c["approximation_error_estimate"]) else {
            return json!({ "available": false, "reason": "no certificate for this spec" });
        };
        let mh = e["value_mEh"].as_f64().unwrap_or(f64::NAN);
        json!({
            "available": mh.is_finite(),
            "kind": "estimate, not a bound",
            "what": "the published CCSD(T) correlation-energy error of the spec's Hamiltonian",
            "value_Ha": mh / 1000.0,
            "budget_Ha": APPROX_BUDGET,
            "within_budget_by_estimate": mh.abs() / 1000.0 <= APPROX_BUDGET,
            "how": e["how"],
            "rigorous_statements_Ha": e["rigorous_statements_Ha"],
        })
    }

    /// `lambda_eff = sqrt((lambda + E0')(lambda - E0'))` (Low et al. 2025 Eq. (11) in this walk's
    /// normalization) at a certified upper bound on the encoded operator's lowest eigenvalue `E0'`.
    fn lambda_eff(run: &Run<'_>, cert: Option<&Value>) -> Result<Value, String> {
        let cert = cert.ok_or_else(|| format!("no certificate for spec {}", run.spec))?;
        let off = &cert["walk_offsets"][run.lane_map];
        if off.is_null() {
            return Err(format!(
                "certificate has no walk offset for lane map {}",
                run.lane_map
            ));
        }
        let f = |v: &Value, what: &str| v.as_f64().ok_or_else(|| format!("certificate: {what}"));
        let e_up = f(&cert["ground_energy_upper_bound"]["E_up"], "E_up")?;
        let identity = f(&cert["identity"]["float"], "identity")?;
        let lambda_spec = f(&cert["lambda"]["float"], "lambda")?;
        let factor = f(&off["error_factor"], "error_factor")?;
        let mut offset = f(&off["float"], "offset")?;
        if off["plus_lambda_decl_minus_lambda"].as_bool() == Some(true) {
            offset += run.lambda - lambda_spec;
        }
        let allowance = factor * run.rounding_error;
        let e0 = e_up - identity + offset + allowance;
        if !(e0 > -run.lambda && e0 <= 0.0) {
            return Err(format!(
                "the bound E0' <= {e0} is not in (-lambda, 0], where lambda_eff is increasing in E0'"
            ));
        }
        let leff = ((run.lambda + e0) * (run.lambda - e0)).sqrt();
        Ok(json!({
            "available": true,
            "formula": "lambda_eff = sqrt((lambda + E0')(lambda - E0')), E0' = E_up - identity + walk_offset + error_factor x rounding_error",
            "source": "Low et al. 2025 (arXiv:2502.15882) Eq. (11), lambda_eff = sqrt(E_gap (2 Lambda - E_gap)), with E_gap = Lambda + E0'",
            "direction": "lambda_eff increases with E0' on [-lambda, 0], so an upper bound on E0' gives an upper bound on lambda_eff",
            "certificate": format!("specs/{}/certificate.json", run.spec),
            "E_up": e_up,
            "identity": identity,
            "walk_offset": offset,
            "offset_allowance": allowance,
            "E0_prime_upper": e0,
            "sector": cert["ground_energy_upper_bound"]["sector"]["covers"],
            "lambda_eff": leff,
            "controlled": view("controlled cost comparison, lambda_eff", leff, EPS_CONTROLLED, run),
            "chemical_accuracy": view("chemical-accuracy resource estimate (Lee et al. 2021 convention), lambda_eff", leff, EPS_PEA, run),
        }))
    }

    /// The spectrum-amplified walk's `lambda_eff` for a run (spec/SPEC-SA.md section 6): ours from
    /// the spec's certificate (`lambda_eff` above, with the run's `lambda_decl` and rounding error),
    /// the paper's (copied into the certificate), and the one a score uses, the larger of the two.
    /// `None` when the certificate gives no bound for this run.
    #[must_use]
    pub fn sa_lambda_eff(run: &Run<'_>) -> Option<(f64, f64, f64)> {
        let cert = certificate(run.spec)?;
        let ours = lambda_eff(run, Some(&cert)).ok()?["lambda_eff"].as_f64()?;
        let theirs = cert["spectral_amplification"]["published"]["lambda_eff"].as_f64()?;
        Some((ours, theirs, ours.max(theirs)))
    }

    /// Low et al. 2025's per-rotation charge is `beta` Toffolis for each of a Givens' two Majorana
    /// blocks (their spin_free_block_encoding_costs.py: `basis_rotations = 4 * (N - 1) * beta` per
    /// SELECT), against this harness's `2 (beta - 2)` per Givens (spec/DESIGN.md section 15).
    pub const LOW_GIVENS_EXTRA: f64 = 4.0;

    /// The `spectral_amplification` block of `score.json` for sos-sa runs.
    #[must_use]
    pub fn sa_view(run: &Run<'_>, givens_per_step: f64) -> Value {
        let Some((ours, theirs, used)) = sa_lambda_eff(run) else {
            return json!({ "available": false, "reason": "no certificate bound for this run; the score uses the worst-case Lambda" });
        };
        let at = |leff: f64| {
            json!({
                "lambda_eff": leff,
                "sigma_pea_1_0_mHa": view("Low et al. 2025 convention: sigma_PEA = 1.0 mHa", leff, EPS_PEA, run),
                "controlled_1_6_mHa": view("this repository's controlled comparison: 1.6 mHa to phase estimation", leff, EPS_CONTROLLED, run),
            })
        };
        let adj = run.toffoli + LOW_GIVENS_EXTRA * givens_per_step;
        json!({
            "available": true,
            "doc": "spec/SPEC-SA.md section 6",
            "Lambda": run.lambda,
            "lambda_eff_ours": ours,
            "lambda_eff_published": theirs,
            "lambda_eff_used": used,
            "rule": "score = lambda_eff_used x C_step x Q_peak with lambda_eff_used = max(ours, published): both are upper bounds (ours from the spec's certificate: a UHF determinant energy of H_spec, the run's lambda_decl and 2 x its rounding error; theirs: the paper's HF-adjusted gap), and the larger is the one a certified claim may use",
            "steps_formula": "ceil(pi lambda_eff / (2 eps_PEA)) walk steps, each one controlled application of BE[H_SA / Lambda - I] and the reflection (Low et al. 2025 Eq. (11), Table V caption)",
            "ours": at(ours),
            "published": at(theirs),
            "used": at(used),
            "qubits_without_pe_register": run.qubits,
            "low2025_givens_charge": {
                "kind": "derived",
                "toffoli_per_step": adj,
                "formula": "C_step + 4 x (executed Givens per step): Low et al. charge beta Toffolis per Majorana block, 2 beta per Givens, where this harness charges 2 (beta - 2); use it to compare with their Cost[BE] under their rotation charge",
            },
        })
    }

    /// The `conventions` block of `score.json`.
    #[must_use]
    pub fn report(run: &Run<'_>) -> Value {
        let cert = certificate(run.spec);
        let mut chem = view(
            "chemical-accuracy resource estimate (Lee et al. 2021 convention)",
            run.lambda,
            EPS_PEA,
            run,
        );
        chem["epsilon_total"] = json!(EPS_TOTAL);
        chem["approximation_error"] = estimate(cert.as_ref());
        let mut eff = lambda_eff(run, cert.as_ref())
            .unwrap_or_else(|why| json!({ "available": false, "reason": why }));
        if eff["available"] == json!(true) {
            eff["chemical_accuracy"]["epsilon_total"] = json!(EPS_TOTAL);
            eff["chemical_accuracy"]["approximation_error"] = chem["approximation_error"].clone();
        }
        json!({
            "version": VERSION,
            "doc": "spec/CONVENTIONS.md",
            "qubits_peak": run.qubits,
            "qubit_convention": "qubits_lee = qubits_peak + 2 ceil(log2(I + 1)) - 1: the phase-estimation control register and its unary-iteration ancillas for I steps, as Lee et al. 2021 count them (App. A item 1, Eq. (46))",
            "worst_case": {
                "lambda": run.lambda,
                "controlled": view("controlled cost comparison", run.lambda, EPS_CONTROLLED, run),
                "chemical_accuracy": chem,
            },
            "lambda_eff": eff,
        })
    }
}

/// One `results.tsv` row. A failed run keeps its row with status `FAIL` and the reason.
#[derive(Clone, Debug, Default)]
pub struct ResultsRow {
    pub unix_time: u64,
    pub commit: String,
    pub spec: String,
    pub family_name: String,
    pub family_axes: String,
    pub lambda: f64,
    pub toffoli: f64,
    pub qubits: u64,
    pub score: f64,
    pub total_toffoli_lit: f64,
    pub samples: usize,
    pub status: String,
    pub note: String,
}

impl ResultsRow {
    /// A row describing a passing evaluation.
    #[must_use]
    pub fn from_eval(ev: &Evaluation) -> Self {
        let axes = &ev.family.family.axes;
        Self {
            spec: ev.facts.spec_id.clone(),
            family_name: ev.family.family.name.clone(),
            family_axes: axes
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(";"),
            lambda: ev.lambda.to_f64(),
            toffoli: ev.toffoli,
            qubits: ev.qubits,
            score: ev.score(),
            total_toffoli_lit: ev.total_toffoli_lit(),
            samples: ev.samples,
            status: "OK".into(),
            ..Self::default()
        }
    }

    fn line(&self) -> String {
        // Untrusted text (family name, spec id, rejection reasons) must not break the row: no
        // control characters (tabs, newlines, terminal escapes), and no `"`, which a CSV reader
        // takes as an opening quote that swallows every later row.
        let clean = |s: &str| -> String {
            s.chars()
                .map(|c| match c {
                    '"' => '\'',
                    c if c.is_control() => ' ',
                    c => c,
                })
                .collect()
        };
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{}\t{:.6e}\t{:.6e}\t{}\t{}\t{}\n",
            self.unix_time,
            clean(&self.commit),
            clean(&self.spec),
            clean(&self.family_name),
            clean(&self.family_axes),
            self.lambda,
            self.toffoli,
            self.qubits,
            self.score,
            self.total_toffoli_lit,
            self.samples,
            clean(&self.status),
            clean(&self.note)
        )
    }
}

/// Appends `row` to `path`, writing the header first when the file is new or empty.
///
/// # Errors
/// Any I/O error.
pub fn append_results_row(path: &Path, row: &ResultsRow) -> std::io::Result<()> {
    let fresh = std::fs::metadata(path).map_or(true, |m| m.len() == 0);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if fresh {
        writeln!(f, "{RESULTS_HEADER}")?;
    }
    f.write_all(row.line().as_bytes())
}
