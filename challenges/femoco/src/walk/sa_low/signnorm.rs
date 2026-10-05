//! Lever `s`: global sign normalisation of the Householder vectors.
//!
//! **Statement.** Every network of an sos-sa spec is a chain `G_(N-2) ... G_1 G_0` with `G_j` on
//! orbitals `(j, j + 1)` and angle `theta_j = 2 pi a_j / 2^beta` (the first entry acts first), so
//! it maps orbital 0 to the hyperspherical vector
//! `u = (c_0, s_0 c_1, s_0 s_1 c_2, ..., s_0 ... s_(N-3) c_(N-2), s_0 ... s_(N-3) s_(N-2))`.
//! Replacing `a_j` by `2^(beta - 1) - a_j` for `j < N - 2` (`theta -> pi - theta`: `c -> -c`,
//! `s -> s`) and `a_(N-2)` by `a_(N-2) + 2^(beta - 1)` (`theta -> theta + pi`: both signs flip)
//! maps every component to its negative, exactly (the new angles are on the same `2^-beta` grid).
//!
//! **Why the operator is unchanged.** A square leaf applies `Z(u, s) = V_u Z_s V_u^dagger =
//! 1 - 2 n(u, s)` (spec/SPEC-SA.md section 2.3), and `n(-u, s) = n(u, s)`. A one-body leaf applies
//! the rotated Majorana `gamma(u)` in each copy; both copies of a one-body lane use the same network
//! (the outer item fixes it), so with the same choice per leaf the product `gamma(-u) gamma(-u)'`
//! equals `gamma(u) gamma(u)'`. Lanes whose copy applies no Majorana see `V^dagger V = I` for any
//! angles. The choice is a function of the leaf, so both passes of a copy agree.
//!
//! **What it buys.** On `li-sa-est-v1` and `reiher-sa-est-v1` the angles at chain positions
//! `0 .. N - 3` lie strictly inside `(0, 2^(beta - 1))` and only the last position uses the full
//! range. Flipping exactly the leaves whose last angle has its top bit set puts every angle of
//! every leaf below `2^(beta - 1)`: the shared angle register is `beta - 1` qubits, not `beta`,
//! and the top-bit corrections of the transitions into and out of the last position vanish.
//!
//! Prior art: the sign freedom of a Householder vector (`H(u) = H(-u)`) is textbook; its use to
//! drop a qubit from a Givens network's angle register was not found in Low et al. 2025, Lee et
//! al. 2021 or von Burg et al. 2021 (checked their angle-register descriptions only).
use crate::spec::sa::SaSpec;
use crate::spec::Network;
use std::sync::Arc;

/// Whether `net` is the chain the derivation needs: rotation `j` on orbitals `(j, j + 1)`.
#[must_use]
pub fn is_chain(net: &Network) -> bool {
    net.rotations
        .iter()
        .enumerate()
        .all(|(j, &(p, q, _))| usize::from(p) == j && usize::from(q) == j + 1)
}

/// The angles of `-u`'s network (see the module notes), on the same `2^beta` grid.
#[must_use]
pub fn flipped(net: &Network) -> Network {
    assert!(is_chain(net), "sign normalisation needs a chain network");
    let unit = 1u64 << net.beta;
    let half = unit / 2;
    let last = net.rotations.len() - 1;
    let rotations = net
        .rotations
        .iter()
        .enumerate()
        .map(|(j, &(p, q, a))| {
            let a = u64::from(a) % unit;
            let b = if j < last {
                (half + unit - a) % unit
            } else {
                (a + half) % unit
            };
            (p, q, b as u32)
        })
        .collect();
    Network {
        beta: net.beta,
        rotations,
    }
}

/// The network a leaf is delivered as under lever `s`: flipped when its last angle has its top
/// bit set (fault 80 flips only the first `N - 2` angles, an inexact network).
#[must_use]
pub fn normalized(net: &Arc<Network>) -> Arc<Network> {
    let Some(&(_, _, a)) = net.rotations.last() else {
        return net.clone();
    };
    let top = 1u32 << (net.beta - 1);
    if !is_chain(net) || (a % (1 << net.beta)) & top == 0 {
        return net.clone();
    }
    let mut f = flipped(net);
    if super::onehot::fault_is(80) {
        let last = f.rotations.len() - 1;
        f.rotations[last].2 = net.rotations[last].2;
    }
    Arc::new(f)
}

/// `spec` with every square and one-body network replaced by [`normalized`]; everything else (the
/// coefficients the lane map is built from) is the spec's own.
#[must_use]
pub fn sign_normalized(spec: &SaSpec) -> SaSpec {
    SaSpec {
        id: spec.id.clone(),
        n: spec.n,
        r: spec.r,
        b: spec.b,
        c: spec.c,
        beta: spec.beta,
        electrons: spec.electrons,
        sos_const: spec.sos_const,
        e: spec.e.clone(),
        e_nets: spec.e_nets.iter().map(normalized).collect(),
        wb: spec.wb.clone(),
        w: spec.w.clone(),
        nets: spec.nets.iter().map(normalized).collect(),
        lambda: spec.lambda.clone(),
        identity: spec.identity.clone(),
        e_sos: spec.e_sos.clone(),
        lambda_flat: spec.lambda_flat.clone(),
        published_lambda_eff: spec.published_lambda_eff,
        sha: spec.sha,
        rounding: spec.rounding.clone(),
        rounding_class: spec.rounding_class.clone(),
        widths: spec.widths.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! Exhaustive and random checks of the identity behind lever `s`, in the spec's own Givens
    //! convention (`G a+_p G^dagger = cos a+_p + sin a+_q`, spec/DESIGN.md section 15): the flipped
    //! network maps orbital 0 to exactly `-u`; the fault-80 network does not.
    use super::{flipped, Network};

    /// The image of orbital 0 under the chain, rotations applied in order.
    fn image(net: &Network) -> Vec<f64> {
        let n = net.rotations.len() + 1;
        let mut v = vec![0.0f64; n];
        v[0] = 1.0;
        let unit = f64::from(1u32 << net.beta);
        for &(p, q, a) in &net.rotations {
            let th = 2.0 * std::f64::consts::PI * f64::from(a) / unit;
            let (c, s) = (th.cos(), th.sin());
            let (vp, vq) = (v[p as usize], v[q as usize]);
            // a+_p -> c a+_p + s a+_q, a+_q -> c a+_q - s a+_p, applied to sum_k v_k a+_k.
            v[p as usize] = c * vp - s * vq;
            v[q as usize] = s * vp + c * vq;
        }
        v
    }

    fn chain(beta: u8, a: &[u32]) -> Network {
        Network {
            beta,
            rotations: a
                .iter()
                .enumerate()
                .map(|(j, &x)| (j as u16, j as u16 + 1, x))
                .collect(),
        }
    }

    fn assert_negated(net: &Network) {
        let (u, w) = (image(net), image(&flipped(net)));
        for (x, y) in u.iter().zip(&w) {
            assert!((x + y).abs() < 1e-12, "{net:?}: {u:?} vs {w:?}");
        }
    }

    #[test]
    fn flip_negates_u_exhaustively_at_n3_beta8() {
        for a0 in 0..256u32 {
            for a1 in 0..256u32 {
                assert_negated(&chain(8, &[a0, a1]));
            }
        }
    }

    #[test]
    fn flip_negates_u_on_random_chains() {
        let mut x = 0x1234_5678_9abc_def1u64;
        for n in 2..=76usize {
            for beta in [6u8, 8, 15, 16] {
                let a: Vec<u32> = (0..n - 1)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        (x % (1 << beta)) as u32
                    })
                    .collect();
                assert_negated(&chain(beta, &a));
            }
        }
    }

    #[test]
    fn flip_keeps_angles_below_half_when_the_data_allow() {
        // Inner angles in (0, 2^(beta-1)), last angle with its top bit set: all below half after.
        let net = chain(8, &[1, 100, 127, 200]);
        let f = flipped(&net);
        assert!(f.rotations.iter().all(|r| r.2 < 128), "{f:?}");
    }

    #[test]
    fn fault_80_is_not_a_negation() {
        // Without the last angle's half turn the image is not -u (nor u) for a generic chain.
        let net = chain(8, &[37, 81, 200]);
        let mut f = flipped(&net);
        f.rotations[2].2 = net.rotations[2].2;
        let (u, w) = (image(&net), image(&f));
        let neg = u.iter().zip(&w).all(|(x, y)| (x + y).abs() < 1e-9);
        let same = u.iter().zip(&w).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(!neg && !same);
    }
}
