//! The ground-energy rounding bound `sos-ground-cs-v1` for `sa-nested-alias-v1` lane maps
//! (spec/SPEC-SA.md section 12). It is used only for an sos-sa spec whose
//! `spec.json` declares the rule; every other spec keeps the 0.1 mHa 1-norm rule unchanged.
//!
//! **What it bounds.** The lane map's rounded tables make the walk encode
//! `H~' = sum_alpha O~_alpha^dagger O~_alpha` in place of the spec's
//! `H' = H_spec - E_SOS = sum_alpha O_alpha^dagger O_alpha`, with
//! `O~_alpha = sqrt(2 lambda_decl p^_alpha) sum_x q^_x M_x` and `O_alpha = sum_x a_x M_x`
//! (spec/SPEC-SA.md sections 3 and 5.2). Write `Delta_alpha = O~_alpha - O_alpha`. For every
//! partition of the generators into `A` and `B` and every `t > 0`:
//!
//! - `alpha` in `A`: `O~^dagger O~ = O^dagger O + O^dagger Delta + Delta^dagger O + Delta^dagger
//!   Delta <= (1 + t) O^dagger O + (1 + 1/t) Delta^dagger Delta` (from `(sqrt t O - Delta /
//!   sqrt t)^dagger (...) >= 0`), and the same with the roles of `O` and `O~` exchanged.
//! - `alpha` in `B`: `|O~^dagger O~ - O^dagger O| <= L_alpha` with
//!   `L_alpha = ||Delta|| (||O~|| + ||O||)`.
//!
//! Summing, with `H'_A <= H'` (every square is positive semidefinite),
//! `H~' <= (1 + t) H' + (1 + 1/t) D_A + L_B` and `H' <= (1 + t) H~' + (1 + 1/t) D_A + L_B`, where
//! `D_A = sum_A ||Delta_alpha||^2 >= ||sum_A Delta^dagger Delta||` and `L_B = sum_B L_alpha`. Both
//! operators conserve the electron number (the one-body generators stay `a` or `a^dagger`
//! because their two inner items keep equal counts, which the rule requires), so the
//! inequalities hold on the certificate's sector, and by min-max for its lowest eigenvalues.
//! With `0 <= E0(H') <= G`, `G = E_up - E_SOS` from the spec's certificate,
//! `|E0(H~') - E0(H')| <= t G + (1 + 1/t) D_A + L_B`, and at the best `t`:
//!
//! `|E0(H~') - E0(H')| <= 2 sqrt(G D_A) + D_A + L_B`.
//!
//! The first-order term couples each generator's rounding to `<O^dagger O>`, whose sum over the
//! generators is the gap `E0(H') <= G` (3.5 / 5.4 Ha), not to `||O||` (up to `lambda_alpha^2`, a
//! share of `2 Lambda` = 117 / 359 Ha): the sum-of-squares structure the 1-norm rule cannot see.
//!
//! **Arithmetic.** `||Delta_alpha|| <= sum_x mult_x |q~_x - a_x|` (`||M_x|| = 1`; `mult` 2 for a
//! square's spin item), `||O~|| <= sum_x mult_x q~_x`, `||O|| <= sum_x mult_x a_x`. Every `a_x^2`
//! and `q~_x^2` is an exact dyadic; each square root is bracketed by integer square roots at
//! 2^-P, and `|sqrt X - sqrt Y| <= |X - Y| / (floor sqrt X + floor sqrt Y)`, rounded up. `A` and
//! `B` are chosen by a float scan (any partition is valid); the totals and the decision
//! `D_A + L_B <= budget` and `4 G D_A <= (budget - D_A - L_B)^2` are exact.
use super::{Dy, SaNestedMap, FAMILY};
use crate::spec::sa::SaSpec;
use crate::spec::Exact;
use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{Signed, Zero};

/// The rule name `spec.json` declares (`rounding.rule`).
pub const RULE: &str = crate::spec::sa::GROUND_RULE;

/// Precision of the square-root brackets: `2^-P` relative to the common exponent.
const P: u32 = 128;

/// The bound and its parts, all exact upper bounds.
#[derive(Clone, Debug)]
pub struct GroundBound {
    /// Upper bound on `|E0(H~') - E0(H')|`, Hartree.
    pub bound: Exact,
    /// `D_A = sum_A ||Delta_alpha||^2`.
    pub d_a: Exact,
    /// `L_B = sum_B ||Delta_alpha|| (||O~_alpha|| + ||O_alpha||)`.
    pub l_b: Exact,
    /// `G = E_up - E_SOS`.
    pub g: Exact,
    /// Generators counted in `B` (linearly), of `generators`.
    pub linear: usize,
    pub generators: usize,
}

impl GroundBound {
    /// Whether the bound is at most `budget`, decided exactly:
    /// `D_A + L_B <= budget` and `4 G D_A <= (budget - D_A - L_B)^2`.
    #[must_use]
    pub fn within(&self, budget: &Exact) -> bool {
        let x = budget.sub(&self.d_a).sub(&self.l_b);
        if x.is_negative() {
            return false;
        }
        Exact::from_int(4).mul(&self.g).mul(&self.d_a) <= x.mul(&x)
    }
}

/// `floor(sqrt(m 2^(2P)))` for `m >= 0`: `sqrt(m / 2^e)` lies in `[s, s + 1) / 2^(e/2 + P)`.
fn isqrt_scaled(m: &BigInt) -> BigInt {
    let mag: BigUint = m.magnitude() << (2 * P);
    BigInt::from_biguint(Sign::Plus, mag.sqrt())
}

/// `ceil(a / b)` for `a >= 0`, `b > 0`.
fn ceil_div(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = (a / b, a % b);
    if r.is_zero() {
        q
    } else {
        q + 1
    }
}

/// One LCU item of a generator: `a^2` and `q~^2` at `2^-ex`, and its multiplicity.
struct Item {
    a2: BigInt,
    q2: BigInt,
    mult: u32,
}

/// Per-generator exact upper bounds at `2^-(ex/2 + P)`: `||Delta||`, `||O~|| + ||O||`.
struct Norms {
    delta: BigInt,
    both: BigInt,
    count: u32,
}

fn norms(items: &[Item], count: u32) -> Norms {
    let (mut delta, mut both) = (BigInt::zero(), BigInt::zero());
    for it in items {
        let (sa, sq) = (isqrt_scaled(&it.a2), isqrt_scaled(&it.q2));
        let den = &sa + &sq;
        // |sqrt q2 - sqrt a2| <= |q2 - a2| / (sa + sq) in units of 2^-(ex/2 + P) after the
        // 2^(2P) scaling: (|q2 - a2| 2^(2P)) / (sa + sq).
        let d = if den.is_zero() {
            // Both below 2^-(ex/2 + P): each root is below one unit.
            BigInt::from(1)
        } else {
            ceil_div(&((&it.q2 - &it.a2).abs() << (2 * P)), &den)
        };
        let m = BigInt::from(it.mult);
        delta += &d * &m;
        both += (sa + sq + 2) * &m;
    }
    Norms { delta, both, count }
}

/// `ceil(sqrt(x))` as an exact dyadic upper bound, for `x >= 0` dyadic.
fn sqrt_up(x: &Exact) -> Result<Exact, String> {
    let e = x
        .dyadic_exponent()
        .ok_or(format!("{FAMILY}: ground bound: not dyadic"))?;
    let e2 = e + (e % 2);
    let m: BigInt = &x.num << (e2 - e);
    let s = isqrt_scaled(&m);
    let s = if &s * &s == (&m << (2 * P)) { s } else { s + 1 };
    Ok(Exact::dyadic(s, e2 / 2 + P))
}

impl SaNestedMap {
    /// The `sos-ground-cs-v1` bound (module docs) for this lane map against `d`, with
    /// `g = E_up - E_SOS` from the spec's certificate.
    ///
    /// # Errors
    /// A different spec, a one-body inner table whose two items have different counts (the
    /// rounded generator would not conserve the electron number), or a non-dyadic value.
    pub fn ground_bound(&self, d: &SaSpec, g: &Exact) -> Result<GroundBound, String> {
        if (d.n, d.r, d.b, d.c) != (self.n, self.r, self.b, self.c) {
            return Err(format!("{FAMILY}: lane map was built for a different spec"));
        }
        if g.is_negative() {
            return Err(format!("{FAMILY}: ground bound: G = {g} is negative"));
        }
        let (u_o, w) = (self.outer_bits(), self.inner_width());
        let lam = Dy::of(&self.lambda_decl)?;
        let abs =
            |v: &[f64]| -> Result<Vec<Dy>, String> { v.iter().map(|&x| Dy::abs_f64(x)).collect() };
        let (e, wv, wb) = (abs(&d.e)?, abs(&d.w)?, abs(&d.wb)?);
        let max_e = |v: &[Dy]| v.iter().map(|x| x.e).max().unwrap_or(0);
        let (ee, ew, eb) = (max_e(&e), max_e(&wv), max_e(&wb));
        // a^2: |e|/4, w^2/8, wB^2/2; q~^2 = lambda * (integer) / 2^(u_o + 2w).
        let q_e = lam.e + u_o + 2 * w;
        let mut ex = (ee + 2).max(2 * ew + 3).max(2 * eb + 1).max(q_e);
        ex += ex % 2;
        let at = |x: &Dy, sq: bool, extra: u32| -> BigInt {
            if sq {
                (&x.m * &x.m) << (ex - extra - 2 * x.e)
            } else {
                &x.m << (ex - extra - x.e)
            }
        };
        let q = |int: BigInt| (&lam.m * int) << (ex - q_e);
        let outer = self.outer.counts(d.outer_items());
        let mut gens: Vec<Norms> = Vec::with_capacity(d.outer_items());
        for (r, er) in e.iter().enumerate() {
            let m = self.inner[r].counts(2);
            if m[0] != m[1] {
                return Err(format!(
                    "{FAMILY}: rule {RULE}: one-body inner table {r} has counts {} and {}; they \
                     must be equal so the rounded generator stays a(u) or a^dagger(u)",
                    m[0], m[1]
                ));
            }
            // rho q~_x^2 = (2 lambda n / 2^u_o) (2 m / 2^w)^2 = lambda 8 n m^2 / 2^(u_o + 2w).
            let q2 = q(BigInt::from(outer[r]) * 8 * m[0] * m[0]);
            let it = |_: u8| Item {
                a2: at(er, false, 2),
                q2: q2.clone(),
                mult: 1,
            };
            // Both spins (the outer spin bit), identical.
            gens.push(norms(&[it(0), it(1)], 2));
        }
        for qi in 0..d.r * d.c {
            let o = d.n + qi;
            let m = self.inner[o].counts(d.b + 1);
            // rho = 2 lambda (2 n) / 2^u_o; spin item q~ = m_b / 2^w, identity 2 m_B / 2^w.
            let n4 = BigInt::from(outer[o]) * 4;
            let mut items: Vec<Item> = (0..d.b)
                .map(|b| Item {
                    a2: at(&wv[qi * d.b + b], true, 3),
                    q2: q(&n4 * m[b] * m[b]),
                    mult: 2,
                })
                .collect();
            items.push(Item {
                a2: at(&wb[qi], true, 1),
                q2: q(&n4 * 4 * m[d.b] * m[d.b]),
                mult: 1,
            });
            gens.push(norms(&items, 1));
        }
        // Float scan for the partition: generators sorted by ||Delta|| / (||O~|| + ||O||)
        // (D_alpha / L_alpha), moved to B while the float total falls.
        let fl = |x: &BigInt| Exact::dyadic(x.clone(), 0).to_f64();
        let unit = 2f64.powi(-i32::try_from(ex / 2 + P).unwrap_or(i32::MAX));
        let gf = g.to_f64();
        let dl: Vec<(f64, f64)> = gens
            .iter()
            .map(|n| {
                let c = f64::from(n.count);
                let dd = fl(&n.delta) * unit;
                (c * dd * dd, c * dd * fl(&n.both) * unit)
            })
            .collect();
        let mut order: Vec<usize> = (0..gens.len()).collect();
        let ratio = |i: usize| dl[i].0 / dl[i].1.max(f64::MIN_POSITIVE);
        order.sort_by(|&x, &y| ratio(y).total_cmp(&ratio(x)).then(x.cmp(&y)));
        let (mut da, mut lb) = (dl.iter().map(|v| v.0).sum::<f64>(), 0.0);
        let (mut best, mut cut) = (f64::INFINITY, 0);
        for k in 0..=order.len() {
            let v = 2.0 * (gf * da.max(0.0)).sqrt() + da.max(0.0) + lb;
            if v < best {
                (best, cut) = (v, k);
            }
            if let Some(&i) = order.get(k) {
                da -= dl[i].0;
                lb += dl[i].1;
            }
        }
        let in_b: std::collections::BTreeSet<usize> = order[..cut].iter().copied().collect();
        let (mut d_int, mut l_int) = (BigInt::zero(), BigInt::zero());
        for (i, n) in gens.iter().enumerate() {
            let c = BigInt::from(n.count);
            if in_b.contains(&i) {
                l_int += &c * &n.delta * &n.both;
            } else {
                d_int += &c * &n.delta * &n.delta;
            }
        }
        let d_a = Exact::dyadic(d_int, ex + 2 * P);
        let l_b = Exact::dyadic(l_int, ex + 2 * P);
        let bound = Exact::from_int(2)
            .mul(&sqrt_up(&g.mul(&d_a))?)
            .add(&d_a)
            .add(&l_b);
        Ok(GroundBound {
            bound,
            d_a,
            l_b,
            g: g.clone(),
            linear: cut,
            generators: gens.len(),
        })
    }
}
