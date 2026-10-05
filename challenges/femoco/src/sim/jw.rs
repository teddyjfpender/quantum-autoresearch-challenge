//! Exact Jordan-Wigner conversion of Majorana monomials to Pauli frames (spec/DESIGN.md
//! section 3). Every encoding compares system operators through this one function.
//!
//! A frame is `w^phase X^x Z^z` with `w = exp(i pi / 4)`, `phase` in `Z/8`, and `X^x Z^z` the
//! tensor product over system qubits `q` of `X_q^{x_q} Z_q^{z_q}` (on each qubit, `Z` acts
//! first). `gamma_{2j} = Z_0 ... Z_{j-1} X_j` and `gamma_{2j+1} = Z_0 ... Z_{j-1} Y_j`, with
//! `Y = i X Z`.
use crate::spec::Monomial;

/// `w^phase X^x Z^z` on `n` qubits; `x` and `z` are bit sets of `ceil(n / 64)` words.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    pub x: Vec<u64>,
    pub z: Vec<u64>,
    pub phase: u8,
}

fn words(n: usize) -> usize {
    n.div_ceil(64)
}

impl Frame {
    #[must_use]
    pub fn identity(n: usize) -> Self {
        Self {
            x: vec![0; words(n)],
            z: vec![0; words(n)],
            phase: 0,
        }
    }

    #[must_use]
    pub fn x_bit(&self, q: usize) -> bool {
        self.x[q / 64] >> (q % 64) & 1 == 1
    }

    #[must_use]
    pub fn z_bit(&self, q: usize) -> bool {
        self.z[q / 64] >> (q % 64) & 1 == 1
    }

    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.x.iter().chain(&self.z).all(|&w| w == 0)
    }

    /// `self * other`: `(w^a X^x1 Z^z1)(w^b X^x2 Z^z2) = w^(a+b) (-1)^|z1 & x2| X^(x1^x2) Z^(z1^z2)`.
    #[must_use]
    pub fn mul(&self, o: &Self) -> Self {
        let anti: u32 = self
            .z
            .iter()
            .zip(&o.x)
            .map(|(a, b)| (a & b).count_ones())
            .sum();
        let x = self.x.iter().zip(&o.x).map(|(a, b)| a ^ b).collect();
        let z = self.z.iter().zip(&o.z).map(|(a, b)| a ^ b).collect();
        let phase = (u32::from(self.phase) + u32::from(o.phase) + 4 * (anti & 1)) % 8;
        Self {
            x,
            z,
            phase: u8::try_from(phase).unwrap_or(0),
        }
    }

    /// True when the operator is Hermitian: `(X^x Z^z)^dagger = (-1)^|x & z| X^x Z^z`, so
    /// `w^p X^x Z^z` is Hermitian iff `2p = 4 |x & z| (mod 8)`.
    #[must_use]
    pub fn is_hermitian(&self) -> bool {
        let xz: u32 = self
            .x
            .iter()
            .zip(&self.z)
            .map(|(a, b)| (a & b).count_ones())
            .sum();
        (2 * u32::from(self.phase)) % 8 == (4 * (xz & 1))
    }

    /// Short human form, e.g. `w^4 X0 Y1 Z3` (Y where both bits are set, ignoring phase).
    #[must_use]
    pub fn describe(&self, n: usize) -> String {
        let mut s = format!("w^{}", self.phase);
        for q in 0..n {
            match (self.x_bit(q), self.z_bit(q)) {
                (true, true) => s.push_str(&format!(" XZ{q}")),
                (true, false) => s.push_str(&format!(" X{q}")),
                (false, true) => s.push_str(&format!(" Z{q}")),
                _ => {}
            }
        }
        if self.is_identity() {
            s.push_str(" I");
        }
        s
    }
}

/// Jordan-Wigner Majorana `gamma_m` on `n` qubits.
fn gamma(m: usize, n: usize) -> Frame {
    let j = m / 2;
    let mut f = Frame::identity(n);
    f.x[j / 64] |= 1 << (j % 64);
    let zlen = if m % 2 == 1 { j + 1 } else { j };
    for q in 0..zlen {
        f.z[q / 64] |= 1 << (q % 64);
    }
    f.phase = if m % 2 == 1 { 2 } else { 0 };
    f
}

/// The exact frame of `i^phase gamma_{m_0} ... gamma_{m_{d-1}}` on `n` system qubits.
///
/// # Errors
/// Indices not strictly increasing or out of range (`>= 2n`), `phase >= 4`, or a product that
/// is not Hermitian (the spec contract requires Hermitian terms).
pub fn monomial_to_frame(m: &Monomial, n: usize) -> Result<Frame, String> {
    if m.phase >= 4 {
        return Err(format!("monomial phase {} is not in 0..4", m.phase));
    }
    let mut f = Frame::identity(n);
    f.phase = 2 * m.phase;
    let mut prev: Option<u16> = None;
    for &g in &m.majoranas {
        if prev.is_some_and(|p| p >= g) {
            return Err(format!(
                "majorana indices not increasing: {:?}",
                m.majoranas
            ));
        }
        if usize::from(g) >= 2 * n {
            return Err(format!("majorana {g} out of range for {n} qubits"));
        }
        prev = Some(g);
        f = f.mul(&gamma(usize::from(g), n));
    }
    if !f.is_hermitian() {
        return Err(format!("monomial {m:?} is not Hermitian"));
    }
    Ok(f)
}
