//! Givens-network conjugation with looked-up angles: `V_n M V_n^dagger` for the network `n`
//! named by a quantum index register, where every network is the same chain of mode pairs and
//! only the angles differ (the df specs' networks, spec/DESIGN.md section 15).
//!
//! **Inverse for free.** `V^dagger` needs every angle negated. Conjugating one Givens rotation
//! by the parity `Z_q` of one of its two modes flips its sign
//! (`Z_q a_q Z_q = -a_q`, so `Z_q G_{pq}(theta) Z_q = G_{pq}(-theta)`), and a chain's modes are
//! two-coloured, so with `Z_S` the product of `Z` over one colour class
//! `V^dagger = Z_S (G_0(theta_0) ... G_{R-1}(theta_{R-1})) Z_S`, applied in reverse order with
//! the *same* angles. `Z_S` is Clifford on system qubits. The angle registers therefore serve
//! both halves: one load per chunk transition instead of two, and a full load (`chunk = R`)
//! needs one lookup per conjugation.
//!
//! **Narrow registers.** A register only needs the bits of the largest angle loaded into it
//! (the harness reads a narrower register as a smaller value, spec/DESIGN.md section 15). The
//! df networks' hyperspherical angles `theta_j = atan2(|u[j+1:]|, u_j)` lie in `[0, pi]` for
//! every rotation but the last, and none of the pinned angles equals `pi` exactly, so those
//! registers are `beta - 1` qubits wide ([`NetworkTable::widths`] measures this per rotation
//! from the data; nothing is assumed).
//!
//! **Loads.** Angles are loaded `chunk` rotations at a time into `chunk` registers (plus the
//! table's flag bits, loaded with the first chunk). The chunk sequence is
//! `C-1, ..., 0` (for `V^dagger`), then `1, ..., C-1` (for `V`): `2C - 1` loads, the first a
//! fresh load ([`qroam_load`] with the plan's block count) and the rest XOR transitions by
//! unary iteration, then one measurement-based erasure. Toffolis per conjugation:
//! `L_first + (2C - 2) E + E(E, erase_h)` with `E` networks; qubits: the slots' widths plus
//! `flag_bits`.
//!
//! Interface for other architectures (the nested DF composition uses the same pieces):
//! build a [`NetworkTable`] (for DF, [`NetworkTable::from_df`]), choose a [`LoadPlan`], and call
//! [`conjugate`] with the index register, the map from table modes to system qubits, a hook
//! that consumes the flag qubits, and the middle operation. [`spin_layer`] is the `F^v` layer
//! of `pi/2` Givens that moves spin-0 modes to spin 1.
use super::lookup::{erase_lookup, put, qroam_load, word, xor, xor_lookup, Qroam, Word};
use crate::circuit::{Builder, Qubit, Reg};
use crate::spec::df::DfSpec;

/// Networks sharing one chain of mode pairs.
pub struct NetworkTable {
    /// Mode pairs `(p, q)` of every network, in `V`'s application order.
    pub modes: Vec<(usize, usize)>,
    pub beta: usize,
    /// `angles[n][j]`: rotation `j` of network `n`, in `0..2^beta`.
    pub angles: Vec<Vec<u32>>,
    /// Register width rotation `j` needs: the bits of its largest angle over all networks.
    pub widths: Vec<usize>,
    /// Flag bits of each network (for example its sign), loaded with the first chunk.
    pub flags: Vec<u64>,
    pub flag_bits: usize,
}

impl NetworkTable {
    /// The df spec's networks in the df baseline's order (one-body `0..N`, then every
    /// leaf's), with one flag bit: `[t_k > 0]` for one-body network `k` (the term's coefficient
    /// is `-t_k / 2`) and `[e_lk < 0]` for a leaf network, so a pair's sign is the XOR of its
    /// networks' flags.
    ///
    /// # Panics
    /// If the networks are not one shared chain.
    #[must_use]
    pub fn from_df(spec: &DfSpec) -> Self {
        let mut angles = Vec::new();
        let mut flags = Vec::new();
        let unit = 1u32 << spec.beta;
        let chain = |n: &crate::spec::Network| -> Vec<(usize, usize)> {
            n.rotations
                .iter()
                .map(|&(p, q, _)| (usize::from(p), usize::from(q)))
                .collect()
        };
        let modes = chain(&spec.t_nets[0]);
        let mut push = |n: &crate::spec::Network, f: bool| {
            assert_eq!(chain(n), modes, "networks must share one chain");
            angles.push(n.rotations.iter().map(|r| r.2 % unit).collect());
            flags.push(u64::from(f));
        };
        for (k, n) in spec.t_nets.iter().enumerate() {
            push(n, spec.t[k] > 0.0);
        }
        for leaf in &spec.leaves {
            for (k, n) in leaf.nets.iter().enumerate() {
                push(n, leaf.e[k] < 0.0);
            }
        }
        let widths = (0..modes.len())
            .map(|j| {
                let top = angles.iter().map(|a: &Vec<u32>| a[j]).max().unwrap_or(0);
                super::arith::bits_for_value(u64::from(top))
            })
            .collect();
        Self {
            modes,
            beta: usize::from(spec.beta),
            angles,
            widths,
            flags,
            flag_bits: 1,
        }
    }

    /// Number of networks `E`.
    #[must_use]
    pub fn networks(&self) -> u64 {
        self.angles.len() as u64
    }

    /// One colour class of the chain's modes: every mode pair has exactly one member in it.
    ///
    /// # Panics
    /// If the mode graph is not bipartite.
    #[must_use]
    pub fn odd_class(&self) -> Vec<usize> {
        let top = self.modes.iter().map(|&(p, q)| p.max(q)).max().unwrap_or(0);
        let mut colour: Vec<Option<bool>> = vec![None; top + 1];
        // Propagate along the pairs until stable (chains converge in one pass).
        loop {
            let mut changed = false;
            for &(p, q) in &self.modes {
                match (colour[p], colour[q]) {
                    (None, None) => {
                        colour[p] = Some(false);
                        colour[q] = Some(true);
                        changed = true;
                    }
                    (Some(c), None) => {
                        colour[q] = Some(!c);
                        changed = true;
                    }
                    (None, Some(c)) => {
                        colour[p] = Some(!c);
                        changed = true;
                    }
                    (Some(a), Some(c)) => assert_ne!(a, c, "mode graph is not bipartite"),
                }
            }
            if !changed {
                break;
            }
        }
        (0..=top).filter(|&m| colour[m] == Some(true)).collect()
    }
}

/// How the angles are loaded.
#[derive(Clone, Copy, Debug)]
pub struct LoadPlan {
    /// Rotations whose angles are held at once.
    pub chunk: usize,
    /// Block count of the first (fresh) load.
    pub first: Qroam,
    /// One-hot bits of the final measurement-based erasure.
    pub erase_h: usize,
}

impl LoadPlan {
    /// Width of each chunk slot: the widest rotation that uses it.
    #[must_use]
    pub fn slots(&self, t: &NetworkTable) -> Vec<usize> {
        let r = t.modes.len();
        let g = self.chunk.clamp(1, r.max(1));
        (0..g)
            .map(|s| (s..r).step_by(g).map(|j| t.widths[j]).max().unwrap_or(1))
            .collect()
    }

    /// Qubits held while rotating: the slots plus `flag_bits`.
    #[must_use]
    pub fn width(&self, t: &NetworkTable) -> usize {
        self.slots(t).iter().sum::<usize>() + t.flag_bits
    }
}

/// Emits `V_n middle V_n^dagger` for the network `n` held in `index` (`n < E` on every lane).
/// Rotation `(p, q)` acts on system modes `(sys(p), sys(q))`. `on_flags` runs right after the
/// first load with the flag qubits; `middle` runs between `V^dagger` and `V`. Nothing is left
/// behind.
pub fn conjugate(
    b: &mut Builder,
    t: &NetworkTable,
    index: &[Qubit],
    plan: &LoadPlan,
    sys: &dyn Fn(usize) -> usize,
    on_flags: &mut dyn FnMut(&mut Builder, &[Qubit]),
    middle: &mut dyn FnMut(&mut Builder),
) {
    let r = t.modes.len();
    let g = plan.chunk.clamp(1, r.max(1));
    let chunks = r.div_ceil(g);
    let e = t.networks();
    let slot_w = plan.slots(t);
    let at: Vec<usize> = slot_w
        .iter()
        .scan(0, |acc, &w| {
            let a = *acc;
            *acc += w;
            Some(a)
        })
        .collect();
    let angle_bits: usize = slot_w.iter().sum();
    let width = angle_bits + t.flag_bits;
    // State i: chunk c, with the flags on the first state only.
    let states: Vec<usize> = (0..chunks).rev().chain(1..chunks).collect();
    let word_of = |n: u64, si: usize| -> Word {
        let mut w = word(width);
        let Some(ang) = usize::try_from(n).ok().and_then(|n| t.angles.get(n)) else {
            return w;
        };
        let c = states[si];
        for (slot, j) in (c * g..((c + 1) * g).min(r)).enumerate() {
            put(&mut w, at[slot], slot_w[slot], u64::from(ang[j]));
        }
        if si == 0 {
            put(&mut w, angle_bits, t.flag_bits, t.flags[n as usize]);
        }
        w
    };
    let odd: Vec<usize> = t.odd_class();
    let z_s = |b: &mut Builder| {
        for &m in &odd {
            let q = b.system(sys(m));
            b.z(q);
        }
    };
    let first = |n: u64| word_of(n, 0);
    let regs_q = qroam_load(b, index, e, width, &first, plan.first);
    let regs: Vec<Reg> = (0..g)
        .map(|s| b.register(&regs_q[at[s]..at[s] + slot_w[s]]))
        .collect();
    on_flags(b, &regs_q[angle_bits..]);
    for (si, &c) in states.iter().enumerate() {
        if si > 0 {
            let diff = |n: u64| xor(&word_of(n, si - 1), &word_of(n, si));
            xor_lookup(b, index, e, &regs_q, &diff);
        }
        let js: Vec<usize> = (c * g..((c + 1) * g).min(r)).collect();
        let inverse = si < chunks;
        if inverse {
            if si == 0 {
                z_s(b);
            }
            for &j in js.iter().rev() {
                let (p, q) = t.modes[j];
                b.givens_modes(sys(p), sys(q), regs[j - c * g]);
            }
            if si == chunks - 1 {
                z_s(b);
                middle(b);
            }
        }
        if !inverse || (si == chunks - 1) {
            // Chunk 0 serves V right after V^dagger (same angles, no reload).
            for &j in &js {
                let (p, q) = t.modes[j];
                b.givens_modes(sys(p), sys(q), regs[j - c * g]);
            }
        }
    }
    let last = states.len() - 1;
    let fin = |n: u64| word_of(n, last);
    erase_lookup(b, index, e, regs_q, &fin, plan.erase_h);
}

/// An `F^v` layer (`v` in `0..4`): `n` Givens `G_{2p, 2p+1}(v pi / 2)` reading one fresh
/// `beta`-qubit register that holds `v 2^(beta - 2)`. `set(b, lo, hi)` XORs `v`'s bits into the
/// register's top two qubits with Cliffords (or ANDs it erases itself) and is called again to
/// clear them. `F` maps each spin-0 mode to its spin-1 partner (as in a DF walk).
pub fn spin_layer(
    b: &mut Builder,
    beta: usize,
    n: usize,
    set: &dyn Fn(&mut Builder, Qubit, Qubit),
) {
    let qs = b.alloc_n(beta);
    let reg = b.register(&qs);
    set(b, qs[beta - 2], qs[beta - 1]);
    for p in 0..n {
        b.givens_modes(2 * p, 2 * p + 1, reg);
    }
    set(b, qs[beta - 2], qs[beta - 1]);
    qs.into_iter().for_each(|q| b.free(q));
}
