//! Pauli frame -> Majorana product, exactly (phase included).
use crate::sim::jw::Frame;
use crate::sim::tracker::LaneFrame;

fn set(v: &mut [u64], q: usize) {
    v[q / 64] |= 1 << (q % 64);
}

fn bit(v: &[u64], q: usize) -> bool {
    v[q / 64] >> (q % 64) & 1 == 1
}

/// Jordan-Wigner `gamma_m` on `n` qubits as a frame (spec/DESIGN.md section 3).
fn gamma(m: usize, n: usize) -> Frame {
    let j = m / 2;
    let mut f = Frame::identity(n);
    set(&mut f.x, j);
    (0..j + m % 2).for_each(|q| set(&mut f.z, q));
    f.phase = if m % 2 == 1 { 2 } else { 0 };
    f
}

fn z_frame(q: usize, n: usize) -> Frame {
    let mut f = Frame::identity(n);
    set(&mut f.z, q);
    f
}

/// Writes the lane frame `w^phase D X^x Z^z` as `w^k gamma_{m_1} ... gamma_{m_d}` with
/// `m_1 < ... < m_d`, returning `(k, [m_i])`.
///
/// # Errors
/// An odd `S` power on a system qubit (`S` is not a Majorana product; not supported with
/// `Givens`).
pub fn frame_to_majoranas(f: &LaneFrame, n: usize) -> Result<(u8, Vec<u16>), String> {
    let mut fr = Frame {
        x: f.x.clone(),
        z: f.z.clone(),
        phase: f.phase % 8,
    };
    for (q, &s) in f.s_pow.iter().enumerate() {
        match s % 4 {
            0 => {}
            2 => fr = z_frame(q, n).mul(&fr), // S^2 = Z, and D sits on the left
            _ => {
                return Err(format!(
                    "tracker: system qubit {q} carries S^{s} at a Givens (only Pauli frames are supported)"
                ))
            }
        }
    }
    // gamma_{2j} has X_j and Z_{<j}; gamma_{2j+1} has X_j Z_j and Z_{<j}. From the top qubit
    // down, `above` is the parity of chosen Majoranas on higher qubits.
    let mut chosen = Vec::new();
    let mut above = false;
    for j in (0..n).rev() {
        let b = bit(&fr.z, j) ^ above;
        let a = bit(&fr.x, j) ^ b;
        if b {
            chosen.push(2 * j + 1);
        }
        if a {
            chosen.push(2 * j);
        }
        above ^= a ^ b;
    }
    chosen.reverse();
    let mut g = Frame::identity(n);
    for &m in &chosen {
        g = g.mul(&gamma(m, n));
    }
    if g.x != fr.x || g.z != fr.z {
        return Err("tracker: internal error converting a frame to Majoranas".into());
    }
    let k = (8 + fr.phase - g.phase) % 8;
    let ms = chosen
        .iter()
        .map(|&m| u16::try_from(m).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((k, ms))
}
