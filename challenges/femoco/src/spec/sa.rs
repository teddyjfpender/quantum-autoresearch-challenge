//! The `sos-sa` spec: the DFTHC + BLISS sum-of-squares Hamiltonian of Low et al. 2025 (Phys. Rev. X
//! 15, 041016), block-encoded for spectral amplification (spec/SPEC-SA.md).
//!
//! With `Et(u) = sum_s n(u, s) - 1 = -(Z(u, 0) + Z(u, 1)) / 2`, `Z(u, s) = G_u Z_{mode s} G_u^dagger`
//! and `G_u` the quantized Givens network that maps spatial orbital 0 to `u` (the df convention,
//! spec/DESIGN.md section 15),
//!
//! `H_spec = sos_const + sum_r e_r Et(u_r) + sum_{r,c} (1/2) (wB_rc + sum_b w_rcb Et(u_rb))^2
//!         = E_SOS + sum_alpha O_alpha^dagger O_alpha`, `E_SOS = sos_const - sum_r |e_r|`,
//!
//! over the generators (spec/SPEC-SA.md section 3):
//! - one-body `(r, s)`: `O = sqrt|e_r| a(u_r, s)` for `e_r >= 0`, `sqrt|e_r| a^dagger(u_r, s)`
//!   otherwise, as the LCU `sqrt|e_r| (M_0 + M_1) / 2` with `M_0 = gamma(u_r, s, 0)` and
//!   `M_1 = +-i gamma(u_r, s, 1)`, rotated single Majoranas;
//! - square `(r, c)`: `O = (wB + sum_b w_b Et(u_b)) / sqrt 2` as the LCU over `M_(b,s) =
//!   sign(w_b) i gamma(u_b, s, 0) gamma(u_b, s, 1)` (weight `|w_b| / 2`) and `M_B = sign(wB) I`
//!   (weight `|wB|`).
//!
//! `Lambda = (1/2) sum_alpha lambda_alpha^2 = sum_r |e_r| + (1/4) sum_rc (|wB_rc| + sum_b |w_rcb|)^2`,
//! the paper's Eq. (33). The loader proves the payload matches its pinned SHA-256 and that
//! `lambda`, `identity`, `E_SOS` and `lambda_flat` recomputed exactly from it equal `spec.json`.
use super::{EncodingSpec, Exact, Network, Rotated, RotatedPart, SystemOp};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"FEMOSAS1";
pub const ENCODING: &str = "sos-sa";
/// The ground-energy rounding rule a spec may declare (spec/SPEC-SA.md section 12).
pub const GROUND_RULE: &str = "sos-ground-cs-v1";

/// The largest rounding budget a `sos-ground-cs-v1` spec may declare: 0.25 mHa, which fits the
/// worst-case coefficient error inside Low et al. 2025's 1.6 mHa total at their rotation bits
/// (spec/SPEC-SA.md section 12.3).
#[must_use]
pub fn max_ground_budget() -> Exact {
    Exact::new(
        num_bigint::BigInt::from(1),
        num_bigint::BigUint::from(4000u32),
    )
}

/// How a lane map's rounding is judged for this spec (spec/SPEC-SA.md section 12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// The harness rule every spec had before: `sum_t |c_t - c^_t| <= 0.1 mHa` (no `rounding`
    /// block in `spec.json`).
    OneNorm,
    /// `sos-ground-cs-v1`: a rigorous bound on the rounded operator's ground-energy error must be
    /// at most `budget`; `e_up` is the certificate's upper bound on `H_spec`'s sector ground
    /// energy, so `G = e_up - E_SOS`.
    Ground { budget: Exact, e_up: Exact },
}

pub struct SaSpec {
    pub id: String,
    /// Spatial orbitals `N`.
    pub n: usize,
    /// DFTHC rank, basis and copies `(R, B, C)` (Low et al. 2025 Eq. (23)).
    pub r: usize,
    pub b: usize,
    pub c: usize,
    /// Rotation bits: network angle `a` means `2 pi a / 2^beta`.
    pub beta: u8,
    /// Electrons of the sector the BLISS shift is for.
    pub electrons: u32,
    pub sos_const: f64,
    /// One-body eigenvalues `e_r` and their networks.
    pub e: Vec<f64>,
    pub e_nets: Vec<Arc<Network>>,
    /// `wB[r * C + c]`.
    pub wb: Vec<f64>,
    /// `w[(r * C + c) * B + b]`.
    pub w: Vec<f64>,
    /// `nets[r * B + b]` maps orbital 0 to `u_rb`.
    pub nets: Vec<Arc<Network>>,
    pub lambda: Exact,
    pub identity: Exact,
    /// `sos_const - sum |e_r|`: `H_spec - E_SOS` is positive semidefinite.
    pub e_sos: Exact,
    /// Sum of the ordered-product coefficients `c_t` (spec/SPEC-SA.md section 3).
    pub lambda_flat: Exact,
    /// The paper's lambda_eff (spec.json `published.lambda_eff`, Low et al. 2025 Table V).
    pub published_lambda_eff: f64,
    pub sha: [u8; 32],
    /// The spec's rounding rule (`spec.json` `rounding`; absent means [`Rounding::OneNorm`]).
    pub rounding: Rounding,
    /// How a lane map's rounding error is judged: `Rigorous` for `*-sa-v1`, Low et al.'s estimated
    /// class for `*-sa-est-v1` (spec.json `rounding_class`, spec/SPEC-SA.md section 14).
    pub rounding_class: super::rounding::RoundingClass,
    /// Tapered rotation widths (`spec.json` `rotation_widths`, spec/SPEC-SA.md section 13): rotation
    /// `j` of every network holds only the top `widths[j]` bits of its `beta`-bit angle, and a
    /// `Givens` at chain position `j` reads its register at that scale and is charged
    /// `2 (widths[j] - 2)`. `None` (every spec without the block): every rotation has `beta` bits.
    pub widths: Option<Vec<u8>>,
}

/// A generator `alpha` of the sum of squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Generator {
    OneBody { r: usize, spin: u8 },
    Square { r: usize, c: usize },
}

fn exact(x: f64) -> Exact {
    Exact::from_f64(x).unwrap_or_else(|_| Exact::zero())
}

fn part(net: &Arc<Network>, spin: u8, majoranas: Vec<u16>) -> RotatedPart {
    RotatedPart {
        network: Arc::clone(net),
        spin: Some(spin),
        majoranas,
    }
}

/// `x^dagger` for a rotated operator: parts reversed, each part's increasing monomial of length
/// `L` picks up `(-1)^(L (L - 1) / 2)`, and `i^p` becomes `i^-p`.
#[must_use]
pub fn adjoint(x: &SystemOp) -> SystemOp {
    match x {
        SystemOp::Rotated(r) => {
            let flips: u8 = r
                .parts
                .iter()
                .map(|p| {
                    let l = p.majoranas.len();
                    u8::from((l * l.saturating_sub(1) / 2) % 2 == 1)
                })
                .sum();
            SystemOp::Rotated(Rotated {
                phase: (4 - r.phase % 4 + 2 * (flips % 2)) % 4,
                parts: r.parts.iter().rev().cloned().collect(),
            })
        }
        SystemOp::Monomial(m) => {
            let l = m.majoranas.len();
            let flip = u8::from((l * l.saturating_sub(1) / 2) % 2 == 1);
            SystemOp::Monomial(super::Monomial {
                phase: (4 - m.phase % 4 + 2 * flip) % 4,
                majoranas: m.majoranas.clone(),
            })
        }
    }
}

/// `x y` for two rotated operators (`y` acts first).
#[must_use]
pub fn product(x: SystemOp, y: SystemOp) -> SystemOp {
    let rot = |o: SystemOp| match o {
        SystemOp::Rotated(r) => r,
        SystemOp::Monomial(m) => {
            assert!(m.majoranas.is_empty(), "sa operators are rotated");
            Rotated {
                phase: m.phase,
                parts: Vec::new(),
            }
        }
    };
    let (x, y) = (rot(x), rot(y));
    SystemOp::Rotated(Rotated {
        phase: (x.phase + y.phase) % 4,
        parts: x.parts.into_iter().chain(y.parts).collect(),
    })
}

impl SaSpec {
    /// Generators in outer-item order: one-body `r` for `r < N` (its spin is a free outer bit),
    /// then square `(r, c)` at `N + r C + c`.
    #[must_use]
    pub fn outer_items(&self) -> usize {
        self.n + self.r * self.c
    }

    /// Inner items of outer item `o`: 2 for a one-body eigenvector, `B + 1` for a square (item
    /// `B` is the identity).
    #[must_use]
    pub fn inner_items(&self, o: usize) -> usize {
        if o < self.n {
            2
        } else {
            self.b + 1
        }
    }

    /// `M_x` of the one-body generator `(r, spin)`: `x = 0` is `gamma(u_r, s, 0)`, `x = 1` is
    /// `i gamma(u_r, s, 1)` for `e_r >= 0` (`a = (gamma_0 + i gamma_1) / 2`) and `-i gamma(u_r, s, 1)`
    /// otherwise (`a^dagger`).
    #[must_use]
    pub fn one_body_op(&self, r: usize, spin: u8, x: usize) -> SystemOp {
        let m = 2 * u16::from(spin);
        let net = &self.e_nets[r];
        SystemOp::Rotated(if x == 0 {
            Rotated {
                phase: 0,
                parts: vec![part(net, spin, vec![m])],
            }
        } else {
            Rotated {
                phase: if self.e[r] >= 0.0 { 1 } else { 3 },
                parts: vec![part(net, spin, vec![m + 1])],
            }
        })
    }

    /// `M_j` of square `(r, c)`: `j < B` is `sign(w_rcb) i gamma(u_rb, s, 0) gamma(u_rb, s, 1) =
    /// -sign(w) Z(u_rb, s)`; `j = B` is `sign(wB_rc) I` (the spin is ignored).
    #[must_use]
    pub fn square_op(&self, r: usize, c: usize, j: usize, spin: u8) -> SystemOp {
        if j == self.b {
            return SystemOp::Rotated(Rotated {
                phase: if self.wb[r * self.c + c] < 0.0 { 2 } else { 0 },
                parts: Vec::new(),
            });
        }
        let m = 2 * u16::from(spin);
        let neg = self.w[(r * self.c + c) * self.b + j] < 0.0;
        SystemOp::Rotated(Rotated {
            phase: if neg { 3 } else { 1 },
            parts: vec![part(&self.nets[r * self.b + j], spin, vec![m, m + 1])],
        })
    }

    /// Exact `(lambda, identity, E_SOS, lambda_flat)` from the stored values (spec/SPEC-SA.md
    /// section 2): `lambda = sum|e| + sum_rc S_rc^2 / 4` with `S = |wB| + sum_b |w_b|`,
    /// `identity = sos_const + sum_rc (wB^2 / 2 + sum_b w_b^2 / 4)`, `E_SOS = sos_const - sum|e|`,
    /// `lambda_flat = 2 lambda - (identity - E_SOS)`.
    #[must_use]
    pub fn exact_parts(
        sos_const: f64,
        e: &[f64],
        wb: &[f64],
        w: &[f64],
        b: usize,
    ) -> (Exact, Exact, Exact, Exact) {
        let (half, quarter) = (Exact::dyadic(1.into(), 1), Exact::dyadic(1.into(), 2));
        let lam_h1 = e.iter().fold(Exact::zero(), |a, &x| a.add(&exact(x).abs()));
        let mut lam = lam_h1.clone();
        let mut ident = exact(sos_const);
        for (k, &x_b) in wb.iter().enumerate() {
            let ws = &w[k * b..(k + 1) * b];
            let s = ws
                .iter()
                .fold(exact(x_b).abs(), |a, &x| a.add(&exact(x).abs()));
            lam = lam.add(&s.mul(&s).mul(&quarter));
            let q = ws
                .iter()
                .fold(Exact::zero(), |a, &x| a.add(&exact(x).mul(&exact(x))));
            ident = ident
                .add(&exact(x_b).mul(&exact(x_b)).mul(&half))
                .add(&q.mul(&quarter));
        }
        let e_sos = exact(sos_const).sub(&lam_h1);
        let flat = lam.add(&lam).sub(&ident.sub(&e_sos));
        (lam, ident, e_sos, flat)
    }
}

impl EncodingSpec for SaSpec {
    fn id(&self) -> &str {
        &self.id
    }
    fn encoding(&self) -> &str {
        ENCODING
    }
    fn spatial_orbitals(&self) -> usize {
        self.n
    }
    /// `Lambda`, the walk's normalization (not the 1-norm of the ordered products, which is
    /// `lambda_flat`).
    fn lambda(&self) -> Exact {
        self.lambda.clone()
    }
    fn identity(&self) -> Exact {
        self.identity.clone()
    }
    fn payload_sha256(&self) -> [u8; 32] {
        self.sha
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn rounding_class(&self) -> super::rounding::RoundingClass {
        self.rounding_class.clone()
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or("sa.bin: truncated")?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f64s(&mut self, n: usize) -> Result<Vec<f64>, String> {
        let s = self.take(8 * n)?;
        let v: Vec<f64> = s
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap_or([0; 8])))
            .collect();
        if v.iter().any(|x| !x.is_finite()) {
            return Err("sa.bin: non-finite value".into());
        }
        Ok(v)
    }
    fn networks(&mut self, count: usize, n: usize, beta: u8) -> Result<Vec<Arc<Network>>, String> {
        let s = self.take(4 * count * (n - 1))?;
        let mut out = Vec::with_capacity(count);
        for net in s.chunks_exact(4 * (n - 1)) {
            let mut rotations = Vec::with_capacity(n - 1);
            for (j, c) in net.chunks_exact(4).enumerate() {
                let a = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                if u64::from(a) >> beta != 0 {
                    return Err(format!("sa.bin: angle {a} has more than {beta} bits"));
                }
                let j = u16::try_from(j).map_err(|e| e.to_string())?;
                rotations.push((j, j + 1, a));
            }
            out.push(Arc::new(Network { beta, rotations }));
        }
        Ok(out)
    }
}

/// Parses `sa.bin` (`FEMOSAS1` v1, spec/SPEC-SA.md section 4): `magic, u32 version, N, R, B, C,
/// beta, electrons, f64 sos_const, f64 e[N], u32 e_angles[N][N-1], f64 wB[R][C], f64 w[R][C][B],
/// u32 angles[R][B][N-1]`. `published_lambda_eff` is left at 0 (the loader fills it).
///
/// # Errors
/// Bad magic or version, out-of-range sizes, non-finite values, trailing bytes.
pub fn parse_payload(id: &str, bytes: &[u8]) -> Result<SaSpec, String> {
    let mut rd = Reader { b: bytes, at: 0 };
    if rd.take(8)? != MAGIC {
        return Err("sa.bin: bad magic".into());
    }
    let version = rd.u32()?;
    let n = rd.u32()? as usize;
    let (r, b, c) = (rd.u32()? as usize, rd.u32()? as usize, rd.u32()? as usize);
    let (beta, electrons) = (rd.u32()?, rd.u32()?);
    if version != 1
        || !(2..=1024).contains(&n)
        || !(3..=32).contains(&beta)
        || !(1..=4096).contains(&r)
        || !(1..=1024).contains(&b)
        || !(1..=4096).contains(&c)
        || r * c > 1 << 20
        || electrons as usize > 2 * n
    {
        return Err(format!(
            "sa.bin: version {version}, N {n}, (R, B, C) ({r}, {b}, {c}), beta {beta}, electrons \
             {electrons} not supported"
        ));
    }
    let beta = u8::try_from(beta).map_err(|e| e.to_string())?;
    let sos_const = rd.f64s(1)?[0];
    let e = rd.f64s(n)?;
    let e_nets = rd.networks(n, n, beta)?;
    let wb = rd.f64s(r * c)?;
    let w = rd.f64s(r * c * b)?;
    let nets = rd.networks(r * b, n, beta)?;
    if rd.at != bytes.len() {
        return Err("sa.bin: trailing bytes".into());
    }
    let (lambda, identity, e_sos, lambda_flat) = SaSpec::exact_parts(sos_const, &e, &wb, &w, b);
    Ok(SaSpec {
        id: id.to_string(),
        n,
        r,
        b,
        c,
        beta,
        electrons,
        sos_const,
        e,
        e_nets,
        wb,
        w,
        nets,
        lambda,
        identity,
        e_sos,
        lambda_flat,
        published_lambda_eff: 0.0,
        sha: Sha256::digest(bytes).into(),
        rounding: Rounding::OneNorm,
        rounding_class: super::rounding::RoundingClass::Rigorous,
        widths: None,
    })
}

fn meta_str<'a>(meta: &'a serde_json::Value, path: &[&str]) -> Result<&'a str, String> {
    let mut v = meta;
    for p in path {
        v = v
            .get(p)
            .ok_or_else(|| format!("spec.json: missing {}", path.join(".")))?;
    }
    v.as_str()
        .ok_or_else(|| format!("spec.json: {} is not a string", path.join(".")))
}

/// Loads `dir/sa.bin` and proves it matches `spec.json` (and `specs/INDEX.json` when present).
///
/// # Errors
/// A hash mismatch, a malformed payload, or exact values that differ from the payload's.
pub fn load(dir: &Path, meta: &serde_json::Value) -> Result<Box<dyn EncodingSpec>, String> {
    let id = meta_str(meta, &["id"])?.to_string();
    let file = meta_str(meta, &["payload", "file"])?;
    let bytes = std::fs::read(dir.join(file)).map_err(|e| format!("spec {id}: {e}"))?;
    let mut spec = parse_payload(&id, &bytes)?;
    let sha = hex::encode(spec.sha);
    if meta_str(meta, &["payload", "sha256"])? != sha {
        return Err(format!(
            "spec {id}: payload sha256 {sha} differs from spec.json"
        ));
    }
    super::df::check_index(dir, &id, &sha)?;
    for (name, v) in [
        ("lambda", &spec.lambda),
        ("identity", &spec.identity),
        ("E_SOS", &spec.e_sos),
        ("lambda_flat", &spec.lambda_flat),
    ] {
        let want = meta_str(meta, &[name, "exact"])?;
        if want != v.to_string() {
            return Err(format!(
                "spec {id}: {name} {v} recomputed from the payload differs from spec.json {want}"
            ));
        }
    }
    spec.published_lambda_eff = meta
        .get("published")
        .and_then(|p| p.get("lambda_eff"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    if meta.get("rounding").is_some() {
        spec.rounding = ground_rule(dir, &id, meta, &spec.e_sos)?;
    }
    spec.rounding_class = super::rounding::from_meta(&id, meta)?;
    // The two rounding additions are spec-scoped and exclusive: no spec may declare both.
    if spec.rounding != Rounding::OneNorm
        && spec.rounding_class != super::rounding::RoundingClass::Rigorous
    {
        return Err(format!(
            "spec {id}: declares both a rounding rule and an estimated rounding class"
        ));
    }
    if let Some(block) = meta.get("rotation_widths") {
        spec.widths = Some(rotation_widths(&spec, &id, block)?);
    }
    Ok(Box::new(spec))
}

/// Reads and proves `spec.json` `rotation_widths.widths` (spec/SPEC-SA.md section 13): `N - 1`
/// widths in `3..=beta` whose largest is `beta`, and every network's angle at position `j` a
/// multiple of `2^(beta - widths[j])` (it carries only its top `widths[j]` bits).
///
/// # Errors
/// A malformed block, a width out of range, or an angle with nonzero low bits.
pub fn rotation_widths(s: &SaSpec, id: &str, block: &serde_json::Value) -> Result<Vec<u8>, String> {
    let bad = |why: String| format!("spec {id}: rotation_widths: {why}");
    let widths: Vec<u8> = block
        .get("widths")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| bad("no widths array".into()))?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|w| u8::try_from(w).ok())
                .ok_or_else(|| bad(format!("{v} is not a width")))
        })
        .collect::<Result<_, _>>()?;
    if widths.len() + 1 != s.n {
        return Err(bad(format!(
            "{} widths for N - 1 = {}",
            widths.len(),
            s.n - 1
        )));
    }
    if let Some(w) = widths.iter().find(|&&w| !(3..=s.beta).contains(&w)) {
        return Err(bad(format!("width {w} not in 3..={}", s.beta)));
    }
    if widths.iter().max() != Some(&s.beta) {
        return Err(bad(format!("the largest width must be beta = {}", s.beta)));
    }
    for net in s.e_nets.iter().chain(&s.nets) {
        for (j, &(p, q, a)) in net.rotations.iter().enumerate() {
            let low = (1u32 << (s.beta - widths[j])) - 1;
            if (usize::from(p), usize::from(q)) != (j, j + 1) || a & low != 0 {
                return Err(bad(format!(
                    "rotation {j} ({p}, {q}) angle {a} is not a {}-bit angle at chain position {j}",
                    widths[j]
                )));
            }
        }
    }
    Ok(widths)
}

/// Reads `spec.json` `rounding` (spec/SPEC-SA.md section 12.2): `rule` must be
/// [`GROUND_RULE`] and `budget.exact` in `(0, max_ground_budget()]`. `E_up` comes from the spec's
/// own `certificate.json`, which must name this spec and carry this `spec.json`'s SHA-256.
fn ground_rule(
    dir: &Path,
    id: &str,
    meta: &serde_json::Value,
    e_sos: &Exact,
) -> Result<Rounding, String> {
    let rule = meta_str(meta, &["rounding", "rule"])?;
    if rule != GROUND_RULE {
        return Err(format!("spec {id}: unknown rounding rule {rule:?}"));
    }
    let budget = Exact::parse(meta_str(meta, &["rounding", "budget", "exact"])?)?;
    if budget <= Exact::zero() || budget > max_ground_budget() {
        return Err(format!(
            "spec {id}: rounding budget {budget} not in (0, {}]",
            max_ground_budget()
        ));
    }
    let read = |f: &str| std::fs::read(dir.join(f)).map_err(|e| format!("spec {id}: {f}: {e}"));
    let spec_sha = hex::encode(Sha256::digest(read("spec.json")?));
    let cert: serde_json::Value = serde_json::from_slice(&read("certificate.json")?)
        .map_err(|e| format!("spec {id}: certificate.json: {e}"))?;
    if meta_str(&cert, &["spec"])? != id || meta_str(&cert, &["spec_json_sha256"])? != spec_sha {
        return Err(format!(
            "spec {id}: certificate.json is not this spec's (spec id or spec.json sha256 differ)"
        ));
    }
    let e_up = cert
        .get("ground_energy_upper_bound")
        .and_then(|g| g.get("E_up"))
        .and_then(serde_json::Value::as_f64)
        .ok_or(format!("spec {id}: certificate.json has no E_up"))?;
    let e_up = Exact::from_f64(e_up)?;
    if e_up < *e_sos {
        return Err(format!("spec {id}: certificate E_up {e_up} below E_SOS"));
    }
    Ok(Rounding::Ground { budget, e_up })
}
