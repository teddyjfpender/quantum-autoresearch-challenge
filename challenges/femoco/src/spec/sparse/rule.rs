//! The sparse expansion rule: (entry, expansion bits) -> signed Jordan-Wigner Majorana monomial.
//!
//! Byte-for-byte the rule of the sparse spec generator (no sparse spec, and no generator, ships
//! in this repository).
//!
//! What it proves: for integrals
//! `(h, eri)` the operator `identity + sum_e sum_x (w_e / 32) expand(e, x)` equals the
//! Jordan-Wigner image of the second-quantized Hamiltonian, and every `expand(e, x)` is a
//! Hermitian unitary.
use super::dyadic::Dyadic;
use crate::spec::Monomial;
use num_bigint::BigInt;

/// Uniform bits that expand one entry into its 32 terms.
pub const EXPANSION_BITS: u32 = 5;

/// A unique integral. One-body: `T'_pq` with `p <= q`. Two-body: `(pq|rs)` with `p <= q`,
/// `r <= s`, `(p, q) <= (r, s)`, one representative per 8-fold symmetry orbit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entry {
    One { p: u8, q: u8, v: f64 },
    Two { p: u8, q: u8, r: u8, s: u8, v: f64 },
}

impl Entry {
    #[must_use]
    pub fn value(&self) -> f64 {
        match *self {
            Entry::One { v, .. } | Entry::Two { v, .. } => v,
        }
    }
}

/// `Q_{pq sigma}` of Lee et al. 2021 Eq. (A4) as `(phase, [a, b])`: `i^phase gamma_a gamma_b`.
#[must_use]
pub fn q_monomial(p: u8, q: u8, sigma: u8) -> (u8, [u16; 2]) {
    let a = 2 * u16::from(p) + u16::from(sigma);
    let b = 2 * u16::from(q) + u16::from(sigma);
    match p.cmp(&q) {
        // X_a Z..Z X_b = -i gamma_{2a+1} gamma_{2b}
        std::cmp::Ordering::Less => (3, [2 * a + 1, 2 * b]),
        // Y_b Z..Z Y_a = i gamma_{2b} gamma_{2a+1}
        std::cmp::Ordering::Greater => (1, [2 * b, 2 * a + 1]),
        // -Z_a = i gamma_{2a} gamma_{2a+1}
        std::cmp::Ordering::Equal => (1, [2 * a, 2 * a + 1]),
    }
}

/// Normal-ordered product of two Majorana monomials (`gamma_j^2 = 1`).
#[must_use]
pub fn multiply(x: &Monomial, y: &Monomial) -> Monomial {
    let inversions = x
        .majoranas
        .iter()
        .map(|a| y.majoranas.iter().filter(|b| a > b).count())
        .sum::<usize>();
    let mut merged: Vec<u16> = x.majoranas.iter().chain(&y.majoranas).copied().collect();
    merged.sort_unstable();
    let mut out: Vec<u16> = Vec::with_capacity(merged.len());
    for m in merged {
        if out.last() == Some(&m) {
            out.pop();
        } else {
            out.push(m);
        }
    }
    let phase = (usize::from(x.phase) + usize::from(y.phase) + 2 * inversions) % 4;
    Monomial {
        phase: u8::try_from(phase).unwrap_or(0),
        majoranas: out,
    }
}

/// Whether `i^phase gamma_{m_0} ... gamma_{m_{d-1}}` (increasing indices) is Hermitian.
#[must_use]
pub fn is_hermitian(m: &Monomial) -> bool {
    let d = m.majoranas.len();
    usize::from(m.phase % 2) == (d * d.saturating_sub(1) / 2) % 2
}

fn mono((phase, maj): (u8, [u16; 2])) -> Monomial {
    Monomial {
        phase,
        majoranas: maj.to_vec(),
    }
}

/// The signed monomial of expansion value `x` (low 5 bits) of `entry`.
///
/// One-body: bit 0 swaps `p, q`; bit 3 is the spin; bits 1, 2, 4 are ignored. Two-body: bit 0
/// swaps `p, q`, bit 1 swaps `r, s`, bit 2 swaps the pairs (after bits 0 and 1), bit 3 is the
/// first pair's spin, bit 4 the second's. When the two factors anticommute their product is
/// multiplied by `i`, which keeps every term Hermitian; the pair-swapped image then carries
/// the opposite sign and the two cancel in `H`, exactly as `Q Q' + Q' Q = 0` does.
#[must_use]
pub fn expand(entry: &Entry, x: u8) -> Monomial {
    let mut m = match *entry {
        Entry::One { p, q, .. } => {
            let (p, q) = if x & 1 != 0 { (q, p) } else { (p, q) };
            mono(q_monomial(p, q, (x >> 3) & 1))
        }
        Entry::Two { p, q, r, s, .. } => {
            let (p, q) = if x & 1 != 0 { (q, p) } else { (p, q) };
            let (r, s) = if x & 2 != 0 { (s, r) } else { (r, s) };
            let (p, q, r, s) = if x & 4 != 0 {
                (r, s, p, q)
            } else {
                (p, q, r, s)
            };
            let a = mono(q_monomial(p, q, (x >> 3) & 1));
            let b = mono(q_monomial(r, s, (x >> 4) & 1));
            let mut m = multiply(&a, &b);
            if !is_hermitian(&m) {
                m.phase = (m.phase + 1) % 4;
            }
            m
        }
    };
    if entry.value() < 0.0 {
        m.phase = (m.phase + 2) % 4;
    }
    m
}

/// Number of distinct ordered index tuples in the 8-fold symmetry orbit of `(p, q, r, s)`.
#[must_use]
pub fn orbit_size(p: u8, q: u8, r: u8, s: u8) -> u32 {
    let mut seen: Vec<[u8; 4]> = (0..8u8)
        .map(|x| {
            let (a, b) = if x & 1 != 0 { (q, p) } else { (p, q) };
            let (c, d) = if x & 2 != 0 { (s, r) } else { (r, s) };
            if x & 4 != 0 {
                [c, d, a, b]
            } else {
                [a, b, c, d]
            }
        })
        .collect();
    seen.sort_unstable();
    seen.dedup();
    u32::try_from(seen.len()).unwrap_or(8)
}

/// Exact entry weight `w_e`: `|T'| * |orbit|` (orbit 1 or 2) for one-body entries and
/// `|V| * |orbit| / 2` for two-body entries. Each of the entry's 32 terms carries `w_e / 32`,
/// so `sum_e w_e = lambda_T + lambda_V` of Lee et al. 2021 Eq. (A10).
#[must_use]
pub fn weight(entry: &Entry) -> Dyadic {
    let v = Dyadic::from_f64(entry.value()).abs();
    match *entry {
        Entry::One { p, q, .. } => v.scale(&BigInt::from(if p == q { 1 } else { 2 }), 0),
        Entry::Two { p, q, r, s, .. } => v.scale(&BigInt::from(orbit_size(p, q, r, s)), 1),
    }
}
