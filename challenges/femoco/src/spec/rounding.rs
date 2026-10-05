//! Rounding-error classes of a spec (spec/SPEC-SA.md section 14; spec/DESIGN.md section 13).
//!
//! Every spec is `Rigorous` unless its `spec.json` carries a `rounding_class` block. Rigorous is the
//! harness's rule since the first release: the lane map's exact 1-norm rounding error
//! `sum_t |c_t - c^_t|` must be at most 0.1 mHa (`score::max_rounding_error`).
//!
//! `EstimatedLow2025` exists for the sos-sa specs `reiher-sa-est-v1` and `li-sa-est-v1` only (the
//! pinned table below; a `rounding_class` on any other spec id is refused at load). It is Low et
//! al. 2025's selection rule for coefficient bits (Phys. Rev. X 15, 041016, App. D and their
//! record's `get_walk_counts_different_bits_adjustments_for_paper_revision.py`, lines 112-143):
//! the standard deviation of the CCSD(T) correlation-energy change under unbiased randomized
//! rounding to `b` bits is modelled as `sigma(b) = 2^(const - b)` mHa, `const` the mean of three
//! fitted values per molecule, and `b` is the smallest integer with `sigma(b) <= 0.83 / sqrt(2)`
//! mHa, i.e. `b = ceil(log2(1 / split) + const)` with `split = 0.83 / sqrt(2)`. That is an
//! **estimate**, not a bound: a run accepted under this class says "within Low et al.'s estimated
//! truncation budget", never "within 0.1 mHa". Rigorous and estimated results are never mixed in
//! one standing (the spec id differs, and score.json and results.tsv label the class).
//!
//! What the lane map must satisfy under this class (`LaneMap::rounding_estimate`):
//! - (rigorous, exact) `lambda_decl` equals the spec's `Lambda`, and every alias count is the floor
//!   or the ceiling of its ideal value `T w_i / sum w` (`T = 2^(k + mu)` lanes of the table), which
//!   is the support of their randomized rounding (their `M_b = floor(M |w_b| / |w|_1)` plus at most
//!   one leftover bin per item);
//! - (the estimate) every alias table resolves at least as finely as their `M = L 2^(b - 1)` bins
//!   over `L` items at the required `b`: `b_equiv = 1 + max{j : L 2^j <= T}` is at least
//!   `ceil(log2(1 / split) + const_coeff)`. Their fit covers the inner tables only (they treat the
//!   outer weights as prepared at machine precision); this class applies the same rule to the
//!   outer table too, which is stricter than theirs;
//! - the spec's rotation bits `beta` meet the same rule with `const_rot` (they do, by construction:
//!   16 and 15 are the authors' own values).
//!
//! **Eq. (D1) variants** (no spec of these ids ships in this repository). The pinned ids
//! `reiher-sa-est-r15-v1`, `li-sa-est-r14-v1` (one fewer rotation bit than the authors') and the
//! new-Hamiltonian ids `reiher-sa-c13-est-v1` / `reiher-sa-c13-est-r15-v1` (their DFTHC rank
//! (10, 27, 13)) are judged by the same exact structural check and the same
//! `sigma(b) = 2^(const - b)`, but the bit rule is their total budget itself, App. D Eq. (D1):
//! `sqrt(sigma_PEA^2 + sigma_coeff(b_coeff)^2 + sigma_rot(beta)^2) + eps_th <= 1.6 mHa` with
//! `sigma_PEA = 1.0` and `eps_th = 0.3` (the Table V caption's values) instead of their script's
//! even split `sigma_coeff, sigma_rot <= 0.83 / sqrt 2` each. Their own script makes the same
//! trade when it is given other bits. The constants
//! stay the FeMoco54 / FeMoco76 means; the (10, 27, 13) solution is one of the three FeMoco54 fits
//! in that mean (its own constants, 7.25 / 14.00, are below it). An `sa-c13` spec keeps
//! `c13-est-v1` on the even split (16 rotation bits, like `reiher-sa-est-v1`).
use serde_json::Value;

/// The class label every result under the estimated class carries.
pub const ESTIMATED_LABEL: &str = "estimated rounding error (Low et al. class)";
/// The `rounding_class.class` string in `spec.json`.
pub const ESTIMATED_CLASS: &str = "estimated-low2025";

/// How the estimated class turns `sigma(b)` into a bit rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EstRule {
    /// Their cost script: each of `sigma_coeff` and `sigma_rot` at most `budget / sqrt 2`, i.e.
    /// `b >= ceil(log2(sqrt 2 / budget) + const)` for both (`*-sa-est-v1`, `reiher-sa-c13-est-v1`).
    EvenSplit,
    /// App. D Eq. (D1) as a whole: `sqrt(sigma_pea^2 + sigma_coeff^2 + sigma_rot^2) + eps_th <=
    /// total` (mHa), the split between coefficient and rotation bits left free.
    EqD1 {
        sigma_pea_mha: f64,
        eps_th_mha: f64,
        total_mha: f64,
    },
}

/// The `rule` block's name for [`EstRule::EqD1`] in `spec.json`.
pub const EQ_D1_RULE: &str = "eq-d1-quadrature";

/// Eq. (D1) at the Table V caption's values: `sigma_PEA = 1.0`, `eps_th = 0.3`, total 1.6 mHa.
pub const EQ_D1: EstRule = EstRule::EqD1 {
    sigma_pea_mha: 1.0,
    eps_th_mha: 0.3,
    total_mha: 1.6,
};

/// The pinned constants of the estimated class.
#[derive(Clone, Debug, PartialEq)]
pub struct EstimatedParams {
    /// Their three fitted `const` values for the coefficient bits (script lines 121-128).
    pub coeff_fits: [f64; 3],
    /// Their three fitted `const` values for the rotation bits.
    pub rot_fits: [f64; 3],
    /// `target_error` of their script: the truncation budget sigma_trunc in mHa (App. D: "leaves a
    /// budget of sigma_trunc <= 0.830 mHa").
    pub budget_mha: f64,
    /// The bit rule ([`EstRule::EvenSplit`] unless the spec is an Eq. (D1) variant).
    pub rule: EstRule,
}

impl EstimatedParams {
    /// `np.mean` of three values, as their script computes it.
    fn mean(v: &[f64; 3]) -> f64 {
        (v[0] + v[1] + v[2]) / 3.0
    }
    #[must_use]
    pub fn coeff_const(&self) -> f64 {
        Self::mean(&self.coeff_fits)
    }
    #[must_use]
    pub fn rot_const(&self) -> f64 {
        Self::mean(&self.rot_fits)
    }
    /// `split = target_error / sqrt(2)`: each of sigma_coeff and sigma_rot gets this share.
    #[must_use]
    pub fn split_mha(&self) -> f64 {
        self.budget_mha / std::f64::consts::SQRT_2
    }
    /// `ceil(log2(1 / split) + const)`, their bit rule.
    #[must_use]
    pub fn required_bits(&self, cnst: f64) -> u32 {
        let b = ((1.0 / self.split_mha()).log2() + cnst).ceil();
        if b <= 0.0 {
            0
        } else {
            b as u32
        }
    }
    /// `sigma(b) = 2^(const - b)` mHa.
    #[must_use]
    pub fn sigma_mha(cnst: f64, bits: u32) -> f64 {
        (cnst - f64::from(bits)).exp2()
    }
}

/// A spec's rounding-error class.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum RoundingClass {
    /// Exact 1-norm rounding error at most 0.1 mHa (every spec but the two below).
    #[default]
    Rigorous,
    /// Low et al. 2025's estimated truncation budget (module docs).
    EstimatedLow2025(EstimatedParams),
}

/// FeMoco54's (Reiher) fitted constants: coeff (7.2, 7.9, 7.9), rot (14.0, 14.3, 14.6).
const REIHER_COEFF: [f64; 3] = [7.2, 7.9, 7.9];
const REIHER_ROT: [f64; 3] = [14.0, 14.3, 14.6];
/// FeMoco76's (Li): coeff (8.6, 8.3, 7.5), rot (13.3, 13.3, 13.1).
const LI_COEFF: [f64; 3] = [8.6, 8.3, 7.5];
const LI_ROT: [f64; 3] = [13.3, 13.3, 13.1];

/// The only specs the estimated class may be declared for, with their pinned constants
/// (`target_error = 0.83`, their script lines 121-137) and bit rule: the authors' even split for
/// `*-sa-est-v1` and `reiher-sa-c13-est-v1`, Eq. (D1) for the one-fewer-rotation-bit variants.
#[must_use]
pub fn pinned(id: &str) -> Option<EstimatedParams> {
    let (coeff_fits, rot_fits, rule) = match id {
        "reiher-sa-est-v1" | "reiher-sa-c13-est-v1" => {
            (REIHER_COEFF, REIHER_ROT, EstRule::EvenSplit)
        }
        "li-sa-est-v1" => (LI_COEFF, LI_ROT, EstRule::EvenSplit),
        "reiher-sa-est-r15-v1" | "reiher-sa-c13-est-r15-v1" => (REIHER_COEFF, REIHER_ROT, EQ_D1),
        "li-sa-est-r14-v1" => (LI_COEFF, LI_ROT, EQ_D1),
        _ => return None,
    };
    Some(EstimatedParams {
        coeff_fits,
        rot_fits,
        budget_mha: 0.83,
        rule,
    })
}

/// The ids [`pinned`] knows, for messages.
pub const PINNED_IDS: &str =
    "reiher-sa-est-v1, li-sa-est-v1, reiher-sa-est-r15-v1, li-sa-est-r14-v1, \
     reiher-sa-c13-est-v1 and reiher-sa-c13-est-r15-v1";

fn triple(v: &Value, key: &str) -> Result<[f64; 3], String> {
    let a = v
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("rounding_class.{key} must be three numbers"))?;
    let mut out = [0.0; 3];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x
            .as_f64()
            .ok_or_else(|| format!("rounding_class.{key} must be three numbers"))?;
    }
    Ok(out)
}

/// The class a spec's `spec.json` declares. No `rounding_class` key: `Rigorous`. Otherwise the id
/// must be in the pinned table and the block must restate its constants exactly.
///
/// # Errors
/// An unknown class, a spec id with no pinned constants, or constants that differ from the pin.
pub fn from_meta(id: &str, meta: &Value) -> Result<RoundingClass, String> {
    let Some(block) = meta.get("rounding_class") else {
        return Ok(RoundingClass::Rigorous);
    };
    let class = block.get("class").and_then(Value::as_str);
    if class != Some(ESTIMATED_CLASS) {
        return Err(format!(
            "spec {id}: unknown rounding_class {class:?} (only {ESTIMATED_CLASS:?} exists)"
        ));
    }
    let pin = pinned(id).ok_or_else(|| {
        format!("spec {id}: the estimated rounding class is pinned for {PINNED_IDS} only")
    })?;
    let rule = match block.get("rule") {
        None => EstRule::EvenSplit,
        Some(r) => {
            let name = r.get("name").and_then(Value::as_str);
            if name != Some(EQ_D1_RULE) {
                return Err(format!(
                    "spec {id}: unknown rounding_class.rule {name:?} (only {EQ_D1_RULE:?})"
                ));
            }
            let num = |k: &str| {
                r.get(k)
                    .and_then(Value::as_f64)
                    .ok_or_else(|| format!("spec {id}: rounding_class.rule.{k} missing"))
            };
            EstRule::EqD1 {
                sigma_pea_mha: num("sigma_pea_mHa")?,
                eps_th_mha: num("eps_th_mHa")?,
                total_mha: num("total_mHa")?,
            }
        }
    };
    let got = EstimatedParams {
        coeff_fits: triple(block, "coeff_const_fits")?,
        rot_fits: triple(block, "rot_const_fits")?,
        budget_mha: block
            .get("sigma_trunc_budget_mHa")
            .and_then(Value::as_f64)
            .ok_or("rounding_class.sigma_trunc_budget_mHa missing")?,
        rule,
    };
    if got != pin {
        return Err(format!(
            "spec {id}: rounding_class constants {got:?} differ from the harness's pin {pin:?}"
        ));
    }
    Ok(RoundingClass::EstimatedLow2025(pin))
}

/// What the estimated procedure found for one lane map (reported in score.json
/// `metrics.rounding_class`).
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub params: EstimatedParams,
    /// Alias widths `(k, mu)` of the outer table and of the inner tables.
    pub outer_bits: (u32, u32),
    pub inner_bits: (u32, u32),
    /// `b_equiv` of the outer table and the smallest over the inner tables that round anything.
    pub b_equiv_outer: u32,
    pub b_equiv_inner: u32,
    /// The spec's rotation bits.
    pub beta: u32,
    /// Tables checked for floor-or-ceiling counts (all of them).
    pub tables: usize,
}

impl Estimate {
    #[must_use]
    pub fn b_coeff(&self) -> u32 {
        self.b_equiv_outer.min(self.b_equiv_inner)
    }
    #[must_use]
    pub fn sigma_coeff_mha(&self) -> f64 {
        EstimatedParams::sigma_mha(self.params.coeff_const(), self.b_coeff())
    }
    #[must_use]
    pub fn sigma_rot_mha(&self) -> f64 {
        EstimatedParams::sigma_mha(self.params.rot_const(), self.beta)
    }
    #[must_use]
    pub fn sigma_trunc_mha(&self) -> f64 {
        self.sigma_coeff_mha().hypot(self.sigma_rot_mha())
    }
    /// Eq. (D1)'s left side `sqrt(sigma_PEA^2 + sigma_coeff^2 + sigma_rot^2) + eps_th` (mHa), for
    /// the [`EstRule::EqD1`] variants; `None` under the even split.
    #[must_use]
    pub fn d1_total_mha(&self) -> Option<f64> {
        match self.params.rule {
            EstRule::EvenSplit => None,
            EstRule::EqD1 {
                sigma_pea_mha,
                eps_th_mha,
                ..
            } => Some(sigma_pea_mha.hypot(self.sigma_trunc_mha()) + eps_th_mha),
        }
    }
    /// The class's rule: under the even split, `b_coeff >= ceil(log2(1/split) + const_coeff)` and
    /// the same for `beta`; under Eq. (D1), its total within its budget.
    #[must_use]
    pub fn accepted(&self) -> bool {
        self.rejection().is_none()
    }
    /// Why a run is refused under this class, if it is.
    #[must_use]
    pub fn rejection(&self) -> Option<String> {
        if let EstRule::EqD1 { total_mha, .. } = self.params.rule {
            let got = self.d1_total_mha().unwrap_or(f64::INFINITY);
            // A table that resolves nothing (b_equiv 0) is refused whatever sigma says.
            if self.b_coeff() == 0 || got.is_nan() || got > total_mha {
                return Some(format!(
                    "estimated rounding class (Eq. (D1)): sqrt(sigma_PEA^2 + sigma_coeff({})^2 + \
                     sigma_rot({})^2) + eps_th = {got:.4} mHa above {total_mha} mHa \
                     (sigma_coeff {:.3}, sigma_rot {:.3} mHa)",
                    self.b_coeff(),
                    self.beta,
                    self.sigma_coeff_mha(),
                    self.sigma_rot_mha()
                ));
            }
            return None;
        }
        let (need_c, need_r) = (
            self.params.required_bits(self.params.coeff_const()),
            self.params.required_bits(self.params.rot_const()),
        );
        if self.b_coeff() < need_c {
            return Some(format!(
                "estimated rounding class: the alias tables resolve b_coeff = {} bits (outer {}, inner {}), \
                 below the {need_c} Low et al.'s rule needs (sigma_coeff {:.3} mHa > {:.3} mHa)",
                self.b_coeff(),
                self.b_equiv_outer,
                self.b_equiv_inner,
                self.sigma_coeff_mha(),
                self.params.split_mha()
            ));
        }
        if self.beta < need_r {
            return Some(format!(
                "estimated rounding class: rotation bits {} below the {need_r} Low et al.'s rule needs",
                self.beta
            ));
        }
        None
    }
}

/// `b_equiv = 1 + max{j : items 2^j <= 2^bits}`: the largest `b` whose `items 2^(b - 1)` bins (Low
/// et al. App. D's `M = (B + 1) 2^(b_coeff - 1)`) are no finer than this table's `2^bits` lanes.
#[must_use]
pub fn b_equiv(items: usize, bits: u32) -> u32 {
    let lanes = 1u128 << bits.min(126);
    let items = items.max(1) as u128;
    let mut j = 0u32;
    while j < 126 && (items << (j + 1)) <= lanes {
        j += 1;
    }
    if items > lanes {
        0
    } else {
        1 + j
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn their_bits_come_out() {
        // Their script prints "Coeff bits: 9.00 Rot bits: 16.00" (FeMoco54) and "9.00 / 15.00" (FeMoco76).
        let r = pinned("reiher-sa-est-v1").unwrap();
        assert_eq!(r.required_bits(r.coeff_const()), 9);
        assert_eq!(r.required_bits(r.rot_const()), 16);
        let l = pinned("li-sa-est-v1").unwrap();
        assert_eq!(l.required_bits(l.coeff_const()), 9);
        assert_eq!(l.required_bits(l.rot_const()), 15);
        // sigma_trunc at their bits: 0.502 / 0.622 mHa.
        let s = |p: &EstimatedParams, b: u32, br: u32| {
            EstimatedParams::sigma_mha(p.coeff_const(), b)
                .hypot(EstimatedParams::sigma_mha(p.rot_const(), br))
        };
        assert!((s(&r, 9, 16) - 0.502).abs() < 1e-3);
        assert!((s(&l, 9, 15) - 0.622).abs() < 1e-3);
    }

    #[test]
    fn eq_d1_variants() {
        let est = |id: &str, b: u32, beta: u32| Estimate {
            params: pinned(id).unwrap(),
            outer_bits: (9, 9),
            inner_bits: (5, 9),
            b_equiv_outer: b,
            b_equiv_inner: b,
            beta,
            tables: 1,
        };
        // Reiher at (9, 15): sqrt(1 + 0.397^2 + 0.616^2) + 0.3 = 1.540 mHa <= 1.6.
        for id in ["reiher-sa-est-r15-v1", "reiher-sa-c13-est-r15-v1"] {
            let e = est(id, 9, 15);
            assert!((e.d1_total_mha().unwrap() - 1.5398).abs() < 1e-3, "{id}");
            assert!(e.accepted(), "{id}");
            // One more bit off either side exceeds 1.6 mHa.
            assert!(
                est(id, 9, 14).rejection().unwrap().contains("Eq. (D1)"),
                "{id}"
            );
            assert!(!est(id, 8, 15).accepted(), "{id}");
            assert!(!est(id, 0, 30).accepted(), "{id}");
        }
        // Li at (9, 14): 1.583 mHa; (9, 13) and (8, 14) exceed.
        let e = est("li-sa-est-r14-v1", 9, 14);
        assert!((e.d1_total_mha().unwrap() - 1.5831).abs() < 1e-3);
        assert!(e.accepted());
        assert!(!est("li-sa-est-r14-v1", 9, 13).accepted());
        assert!(!est("li-sa-est-r14-v1", 8, 14).accepted());
        // The even-split specs keep their rule: 15 rotation bits fail on Reiher, 16 pass.
        assert!(est("reiher-sa-est-v1", 9, 16).accepted());
        assert!(!est("reiher-sa-est-v1", 9, 15).accepted());
        assert!(est("reiher-sa-est-v1", 9, 16).d1_total_mha().is_none());
        assert!(est("reiher-sa-c13-est-v1", 9, 16).accepted());
        assert!(!est("reiher-sa-c13-est-v1", 9, 15).accepted());
        assert!(pinned("reiher-sa-gb-r15-v1").is_none());
    }

    #[test]
    fn b_equiv_matches_their_bins() {
        // (B + 1) = 28 items: 2^14 lanes hold 28 x 2^9 = 14,336 bins, not 28 x 2^10: b = 10.
        assert_eq!(b_equiv(28, 14), 10);
        assert_eq!(b_equiv(28, 13), 9);
        assert_eq!(b_equiv(58, 15), 10);
        assert_eq!(b_equiv(324, 18), 10);
        assert_eq!(b_equiv(2, 1), 1);
        assert_eq!(b_equiv(3, 1), 0);
    }
}
