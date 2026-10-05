//! The reference operator as `w^k gamma(u_1) ... gamma(u_d)`.
use super::{cos_sin, rotate, Tv};
use crate::spec::{Monomial, Rotated, SystemOp};

fn monomial_vectors(m: &Monomial, n: usize) -> Result<Vec<Tv>, String> {
    check_increasing(&m.majoranas, n)?;
    Ok(m.majoranas
        .iter()
        .map(|&a| Tv::basis(usize::from(a), n))
        .collect())
}

fn check_increasing(ms: &[u16], n: usize) -> Result<(), String> {
    if ms.windows(2).any(|w| w[0] >= w[1]) || ms.iter().any(|&a| usize::from(a) >= 2 * n) {
        return Err(format!(
            "reference op: Majoranas {ms:?} not strictly increasing below {}",
            2 * n
        ));
    }
    Ok(())
}

fn rotated_vectors(r: &Rotated, n: usize) -> Result<Vec<Tv>, String> {
    let mut out = Vec::new();
    for part in &r.parts {
        check_increasing(&part.majoranas, n)?;
        let beta = part.network.beta;
        if !(1..=32).contains(&beta) {
            return Err(format!("reference op: rotation bits {beta} out of range"));
        }
        let map = |p: u16| match part.spin {
            Some(s) => 2 * usize::from(p) + usize::from(s),
            None => usize::from(p),
        };
        let rots: Vec<(usize, usize, f64, f64)> = part
            .network
            .rotations
            .iter()
            .map(|&(p, q, a)| {
                let (c, s) = cos_sin(u64::from(a), beta);
                (map(p), map(q), c, s)
            })
            .collect();
        if rots.iter().any(|&(p, q, _, _)| p >= q || q >= n) {
            return Err(format!(
                "reference op: a network rotation leaves modes 0..{n}"
            ));
        }
        for &a in &part.majoranas {
            let mut t = Tv::basis(usize::from(a), n);
            for &(p, q, c, s) in &rots {
                rotate(&mut t.v, p, q, c, s);
            }
            out.push(t);
        }
    }
    Ok(out)
}

/// `(k, [u_i])` with the reference equal to `w^k gamma(u_1) ... gamma(u_d)`; `None` (a
/// control-0 lane) is the identity.
///
/// # Errors
/// A malformed reference (indices out of range or not increasing).
pub fn reference_vectors(r: Option<&SystemOp>, n: usize) -> Result<(u8, Vec<Tv>), String> {
    match r {
        None => Ok((0, Vec::new())),
        Some(SystemOp::Monomial(m)) => Ok(((2 * m.phase) % 8, monomial_vectors(m, n)?)),
        Some(SystemOp::Rotated(r)) => Ok(((2 * r.phase) % 8, rotated_vectors(r, n)?)),
    }
}
