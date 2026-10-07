//! One-hot RPREP (`sa-toff` lever `u`, for unary): the network index is written one-hot, and each Givens
//! angle is fanned out from it by CNOTs into a single angle register just before its rotation.
//!
//! Caesura et al., arXiv:2501.06165 v3, App. D and Fig. 10(a-c) (a unary register feeding many
//! rotations by Clifford fan-out), and Low et al. 2026, arXiv:2605.30455, section E.3.
//!
//! **Write.** The ragged unary iteration of [`super::inner::ragged_read`] visits every network
//! index value `v` of a lane exactly once (`R` square rows of `B`, then the one-body rows:
//! `E = R B + N` leaves). Instead of XORing `v`'s whole angle word (the sum of the rotation
//! widths, 743 / 1,051 qubits at 15 / 14 bits) into a register at each leaf, the leaf's flag is
//! copied into one qubit of an `E`-qubit one-hot register `h`: `h[e] = [v = v_e]` for the
//! lane's `v`. Same iteration, same Toffolis (`E - 1` plus the row level).
//!
//! **Fan-out.** Rotation `j`'s angle `a_j(v)` is `sum_e h[e] a_j(v_e)`: CNOTs from `h[e]` into
//! bit `k` of a `w_j`-qubit angle register for every `e` whose angle has bit `k` set. Going from
//! rotation `j` to `j'` XORs `a_j(v_e) ^ a_j'(v_e)` instead, so the register moves through the
//! chain's angles one transition at a time (`V^dagger` in reverse order, then `V` forward) and
//! is unloaded to `|0>` (and reset) after each half, so it is not live across the Majorana.
//! Only Cliffords: nothing is charged, and one angle register (the widest rotation, 15 / 14
//! qubits) replaces the angle word. The register reads by each `Givens` is the prefix of the
//! rotation's own width (spec/DESIGN.md section 15: a narrower register is allowed at the same
//! charge).
//!
//! **Erase.** Every one-hot qubit is measured in the X basis. Outcomes `m` leave the phase
//! `(-1)^(m[e(v)])` on a lane whose index is leaf `e(v)` (nothing on a non-leaf lane, where the
//! register is empty): a phase table over the index with one bit per leaf, cancelled by the same
//! fixup the angle word's erasure uses (`qroam::fixup`: its cost depends on the index range, not
//! on the data), so RPREP^dagger costs what it did.
//!
//! **In-range iteration** (lever `r`, with `u`). Every lane's index is in range except a
//! square's identity item `lo = B`; iterating as if it were (a node with only a left child passes
//! its control down instead of ANDing it with the negated index bit, as `sa-pareto`'s lean
//! layout does) costs `E - 1` Toffolis instead of the ragged tree's `E - 1` plus its left-only
//! nodes, and sends the identity item to the leaf [`leaf_in_range`] gives it. That lane then
//! rotates by that leaf's angles around an identity Majorana (`V I V^dagger = I`), and the
//! erasure uses the same map ([`lane_leaf`]).
//!
//! **Split one-hot** (levers `v` / `w`: `G = 2` / `3` groups). The one-hot is `E` qubits,
//! and no register from which every angle bit is a CNOT fan-out can be narrower than the GF(2)
//! affine rank of the leaves' angle words (`tests_combo::onehot_rank`: 323 / 930 on Reiher / Li,
//! so the one-hot is within one qubit of that bound). Below it a fan-out needs Toffolis. With a
//! split the leaves fall into `G` groups of `E1 = ceil(E / G)` (group `e / E1`), sharing one
//! `E1`-qubit one-hot `h[e mod E1]` plus a one-hot group bit `g_i = [e / E1 = i]` for each group
//! `i >= 1` (group 0 has none). Rotation `j`'s angle is `L_j(h) ^ sum_i g_i D_ij(h)`, where `L_j`
//! fans out group 0's angles and `D_ij` the difference between group `i`'s and group 0's angles at
//! the same slot. A transition moves `L` by CNOTs as before, and each `D_i` by one Toffoli
//! `(g_i, s, reg[k])` per register bit `k` whose difference changes, with `s` one scratch qubit
//! holding the parity `(D_ij ^ D_ij')_k(h)` (CNOTs, undone after; a single-slot parity uses `h`
//! directly). The erasure measures `h` and the group bits and cancels the phase table (bits
//! `e mod E1` and `E1 + i - 1` for group `i`) by the same fixup.
//!
//! The one-hot is allocated and freed inside the copy; the angle register is declared before it
//! (a copy may not declare registers) and returned to `|0>` by the last transition, then reset
//! (`R` at depth 0, so the harness stops counting it live until the next copy's first load).
use super::qroam::{self, Split};
use super::tables::{leaf_in_range, SaTables};
use crate::circuit::{Bit, Builder, Op, OperationType, Qubit, Reg};
use crate::walk::common::unary::{erase_and, iterate};
use crate::walk::shared::lookup::{word, Word};

// Deliberate faults for the mutant tests (`tests_c1`): 0 none, 1 no erasure fixup, 2 the
// fixup with the leaf map shifted by one, 3 the angle register not unloaded, 4 the first
// transition of `V` skipped, 5 a split one-hot's group corrections skipped, 6 a split one-hot's
// group bits not written; 7 and 8 (`toff.rs` lever `k`) the gated re-read's
// comparison phase dropped, and its outcomes left out of the final fixup; 9 and 10 the same
// for lever `j`'s inner re-read; 11 and 12 (lever `y`) `id`'s gated
// recompute dropped and the classical `pass` never toggled; 20 and 21 (lever `z`) the paired
// correction's `X Y` fan-out skipped, and its two parities swapped between the group bits;
// 22 and 23 (lever `f`) the class flags left out of the erasure fixup, and class 0's slots
// shifted by one; 24 (lever `a`) bit 0's measured-unload phase skipped; 25 and 26 (lever `b`)
// the checkpoint clear skipped, and its identity fix skipped; 30-33 (lever `H`, itemhot.rs) and
// 34 (lever `K`: the `is_ob` parity of the clear skipped).
#[cfg(test)]
thread_local! {
    pub(super) static FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Tests only: forces lever `o`'s split depths `(square, one-body)`.
    pub(super) static FORCE_PACK: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Tests only: forces lever `C`'s one-body split depth.
    pub(super) static FORCE_S_OB: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Tests only: forces lever `.g`'s one-body split depths (one per one-body row).
    pub(super) static FORCE_GRAFT_DEPTHS: std::cell::RefCell<Option<Vec<usize>>> = const { std::cell::RefCell::new(None) };
}

fn fault() -> u8 {
    #[cfg(test)]
    {
        FAULT.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        0
    }
}

/// Whether the mutant fault `k` is set (always false outside tests).
#[must_use]
pub fn fault_is(k: u8) -> bool {
    fault() == k
}

/// Dense leaf number of network index value `v = lo | hi << k_i` (`None` for a value the
/// ragged iteration does not visit: the identity item, padding).
#[must_use]
pub fn leaf_of(t: &SaTables<'_>, v: u64) -> Option<usize> {
    let s = t.spec;
    let row = 1u64 << t.k_i;
    let (hi, lo) = ((v >> t.k_i) as usize, (v & (row - 1)) as usize);
    if hi < s.r {
        return (lo < s.b).then_some(hi * s.b + lo);
    }
    let r = (hi - s.r) << t.k_i | lo;
    (r < s.n).then_some(s.r * s.b + r)
}

/// The one-hot register: `q[slot]` with `slot = e` (no group bits), or with a split one-hot
/// (levers `v`, `w`) `slot = e mod e1` and group bit `g[i - 1] = [e / e1 = i]` for `i >= 1`.
pub struct Hot {
    pub q: Vec<Qubit>,
    pub g: Vec<Qubit>,
    pub e1: usize,
    /// Lever `z`: group bits are corrected in pairs, one Toffoli per pair
    /// and register bit ([`transition`]), instead of one per group bit and register bit.
    pub paired: bool,
    /// Lever `f`: the folded layout ([`Fold`]); `None` is the contiguous
    /// layout (slot `e mod e1`, group `e / e1`).
    pub fold: Option<Fold>,
    /// Lever `Z` (with `z` and an odd number of group bits >= 3): the last pair
    /// and the odd group bit form a triple whose corrections are shared over pairs of register
    /// bits, three Toffolis per two bits ([`triple`]) instead of two per bit: `(G - 1) / 2`
    /// Toffolis per corrected bit on average, the local dimension bound.
    pub shared: bool,
    /// Lever `Y` (with `Z`): the triple's shared product goes into both register
    /// bits by a CNOT sandwich (`k2 ^= k1; k1 ^= v1; k2 ^= k1`) instead of through a scratch qubit
    /// (the triangle form): same Toffolis, one qubit fewer at the transition.
    pub sandwich: bool,
    /// Lever `C`: the in-place record of an aligned-class write ([`write_classes`]),
    /// which [`erase_classes`] undoes by measurement.
    pub rec: Option<std::rc::Rc<ClassRec>>,
}

/// One class of [`ClassRec`]: `(member virtual rows, base slot, low splits)`; member `j` is
/// group `j` and member 0's qubit holds the class flag.
pub type ClassSplits = (Vec<usize>, usize, Vec<(usize, usize, usize)>);

/// What [`write_classes`] built, so that [`erase_classes`] can undo it exactly.
#[derive(Debug)]
pub struct ClassRec {
    /// Row-expansion splits `(node row, child row, hi bit)`; the first is a copy.
    row_splits: Vec<(usize, usize, usize)>,
    /// Sub-row splits of the split one-body rows: `(node vrow, child vrow, lo bit)`.
    sub_splits: Vec<(usize, usize, usize)>,
    /// Per class.
    classes: Vec<ClassSplits>,
    /// The row qubits, then the extra sub-row qubits; `vq[v]` is virtual row `v`'s qubit.
    vq: Vec<Qubit>,
    /// Row `hv`'s qubit index into `vq` (its first sub-row).
    row_v: Vec<usize>,
    /// `C` with `E`: the slots are a caller's register that stays allocated, so
    /// the erasure measures with `hmr_keep` and ends lifetimes with `reset_keep`.
    keep: bool,
    /// Lever `.g`: each graft with its host class's base slot and its length.
    grafts: Vec<(Graft, usize, usize)>,
}

/// Lever `f`, the folded split one-hot. Whole square rows are grouped in
/// classes of `G` rows, one row per group; a class's rows share the slots `base_m + lo`, and its
/// rows set the class flag `F_m` and their group bit at the row level of the write. Each class
/// then costs one iteration over `lo` under `F_m` (`B` leaves), not one per row. The remaining
/// leaves (left-over square rows and the one-body rows) are placed one at a time, contiguously
/// over the groups, as without the lever. The class flags are measured after the write; their
/// outcomes join the one-hot's erasure fixup (`F_m` is a function of the row).
#[derive(Clone, Debug)]
pub struct Fold {
    /// `class_of_row[hv]`: `Some((m, group))` for a folded square row.
    pub class_of_row: Vec<Option<(usize, usize)>>,
    /// First slot of class `m`.
    pub base: Vec<usize>,
    /// `(slot, group, class)` of each leaf.
    pub place: Vec<(usize, usize, Option<usize>)>,
    /// `leaf_at[group][slot]`.
    pub leaf_at: Vec<Vec<Option<usize>>>,
    /// The class flags' X outcomes (set by the write).
    pub flags: Vec<Bit>,
}

impl Hot {
    /// The slot and group of leaf `e`.
    #[must_use]
    pub fn place(&self, e: usize) -> (usize, usize) {
        match &self.fold {
            Some(f) => (f.place[e].0, f.place[e].1),
            None => (e % self.e1, e / self.e1),
        }
    }

    /// The leaf at `group`, `slot` (`None`: no leaf), for `n` leaves.
    #[must_use]
    pub fn leaf(&self, group: usize, slot: usize, n: usize) -> Option<usize> {
        match &self.fold {
            Some(f) => f.leaf_at[group][slot],
            None => (group * self.e1 + slot < n).then_some(group * self.e1 + slot),
        }
    }
}

/// Number of leaves `E = R B + N` (the one-hot's width).
#[must_use]
pub fn leaves(t: &SaTables<'_>) -> usize {
    t.spec.r * t.spec.b + t.spec.n
}

/// The value rotation `j`'s register holds for leaf `e`: its angle mod `2^beta`, or on a tapered
/// spec its top `widths[j]` bits ([`super::tables::stored_angle`], the same value the angle word
/// stores; the register is the `t.widths[j]`-qubit prefix).
#[must_use]
pub fn angle(t: &SaTables<'_>, e: usize, j: usize) -> u64 {
    let s = t.spec;
    let net = if e < s.r * s.b {
        &s.nets[e]
    } else {
        &s.e_nets[e - s.r * s.b]
    };
    super::tables::stored_angle(s, j, net.rotations[j].2)
}

/// Width of the shared angle register: the widest rotation.
#[must_use]
pub fn register_bits(t: &SaTables<'_>) -> usize {
    t.widths.iter().copied().max().unwrap_or(1)
}

/// Declares one `Givens` register per rotation over the prefix of `angle_q` of its width (call
/// before the nested block).
pub fn registers(b: &mut Builder, t: &SaTables<'_>, angle_q: &[Qubit]) -> Vec<Reg> {
    t.widths
        .iter()
        .map(|&w| b.register(&angle_q[..w]))
        .collect()
}

fn unit(bits: usize, e: Option<usize>) -> Word {
    let mut w = word(bits);
    if let Some(e) = e {
        w[e / 64] |= 1 << (e % 64);
    }
    w
}

/// The leaf a lane with network index `v` reaches: `v`'s own (the ragged iteration), or, with
/// the in-range iteration, `v` with every bit a left-only node does not read cleared.
#[must_use]
pub fn lane_leaf(t: &SaTables<'_>, v: u64, in_range: bool) -> Option<usize> {
    if !in_range {
        return leaf_of(t, v);
    }
    let row = 1u64 << t.k_i;
    let hv = leaf_in_range(v >> t.k_i, t.h, t.rows());
    let lo = leaf_in_range(v & (row - 1), t.k_i, t.row_len(hv));
    leaf_of(t, hv * row + lo)
}

/// Allocates the one-hot register and writes the lane's leaf into it: an iteration over `hi`
/// (the rows) and, inside each row, over `lo` up to the row's length, ragged or in range.
pub fn write(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    in_range: bool,
    groups: u8,
) -> Hot {
    write_into(b, t, lo, hi, in_range, groups, None)
}

/// [`write`] into a one-hot register allocated by the caller (lever `E`: the register outlives
/// the copy because the `Givens` registers over its qubits are declared before the nested block).
pub fn write_into(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    in_range: bool,
    groups: u8,
    pre: Option<&[Qubit]>,
) -> Hot {
    let e = leaves(t);
    let hot = if let Some(q) = pre {
        assert!(
            groups <= 1 && q.len() == e,
            "a caller's register is an unsplit one-hot"
        );
        Hot {
            q: q.to_vec(),
            g: Vec::new(),
            e1: e,
            paired: false,
            fold: None,
            shared: false,
            sandwich: false,
            rec: None,
        }
    } else if groups > 1 {
        let e1 = e.div_ceil(usize::from(groups));
        let q = b.alloc_n(e1);
        let g = b.alloc_n(usize::from(groups) - 1);
        Hot {
            q,
            g,
            e1,
            paired: false,
            fold: None,
            shared: false,
            sandwich: false,
            rec: None,
        }
    } else {
        Hot {
            q: b.alloc_n(e),
            g: Vec::new(),
            e1: e,
            paired: false,
            fold: None,
            shared: false,
            sandwich: false,
            rec: None,
        }
    };
    let row = 1u64 << t.k_i;
    let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
        let len = t.row_len(hv);
        let mut inner = |b: &mut Builder, v: u64, f: Qubit| {
            let e = leaf_of(t, hv * row + v).expect("a visited value is a leaf");
            let (slot, group) = hot.place(e);
            b.cx(f, hot.q[slot]);
            if group > 0 && !fault_is(6) {
                b.cx(f, hot.g[group - 1]);
            }
        };
        if in_range {
            node_in_range(b, Some(flag), lo, 0, len, &mut inner);
        } else {
            iterate(b, Some(flag), lo, len, &mut inner);
        }
    };
    if in_range {
        node_in_range(b, None, hi, 0, t.rows(), &mut leaf);
    } else {
        iterate(b, None, hi, t.rows(), &mut leaf);
    }
    hot
}

/// The folded layout of lever `f` for `groups` groups ([`Fold`]): `floor(R / G)` classes of `G`
/// consecutive square rows; the other leaves placed contiguously over the groups after the
/// classes' slots. Returns the layout (without outcomes) and `e1`.
#[must_use]
pub fn fold_layout(t: &SaTables<'_>, groups: usize) -> (Fold, usize) {
    let s = t.spec;
    let n = leaves(t);
    let classes = s.r / groups;
    let rows = t.rows() as usize;
    let mut class_of_row = vec![None; rows];
    let mut base = Vec::with_capacity(classes);
    for m in 0..classes {
        base.push(m * s.b);
        for g in 0..groups {
            class_of_row[m * groups + g] = Some((m, g));
        }
    }
    let folded = classes * s.b;
    let rest: Vec<usize> = (0..n)
        .filter(|&e| e >= s.r * s.b || class_of_row[e / s.b].is_none())
        .collect();
    let per = rest.len().div_ceil(groups);
    let e1 = folded + per;
    let mut place = vec![(0, 0, None); n];
    let mut leaf_at = vec![vec![None; e1]; groups];
    for e in 0..s.r * s.b {
        if let Some((m, g)) = class_of_row[e / s.b] {
            let slot = base[m] + e % s.b;
            place[e] = (slot, g, Some(m));
            leaf_at[g][slot] = Some(e);
        }
    }
    for (k, &e) in rest.iter().enumerate() {
        let (g, slot) = (k / per, folded + k % per);
        place[e] = (slot, g, None);
        leaf_at[g][slot] = Some(e);
    }
    (
        Fold {
            class_of_row,
            base,
            place,
            leaf_at,
            flags: Vec::new(),
        },
        e1,
    )
}

/// [`write`] with the folded layout (lever `f`, in-range iteration): one iteration over the rows
/// sets each folded row's class flag and group bit, or walks a non-folded row's leaves; then
/// each class's flag drives one iteration over `lo`; the class flags are X-measured.
pub fn write_folded(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    groups: u8,
) -> Hot {
    write_folded_early(b, t, lo, hi, groups, false)
}

/// [`write_folded`]; with `early` (lever `Q`)
/// each class flag is X-measured as soon as its class's iteration over `lo`
/// is done instead of after the last class, so at most one flag is live in the class phase. The
/// outcomes and their fixup are the same (a flag is a function of the row whenever it is
/// measured).
pub fn write_folded_early(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    groups: u8,
    early: bool,
) -> Hot {
    let (mut fold, e1) = fold_layout(t, usize::from(groups));
    let q = b.alloc_n(e1);
    let g = b.alloc_n(usize::from(groups) - 1);
    let flags = b.alloc_n(fold.base.len());
    let row = 1u64 << t.k_i;
    {
        let place = &fold.place;
        let class_of_row = &fold.class_of_row;
        let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
            if let Some((m, grp)) = class_of_row[hv as usize] {
                b.cx(flag, flags[m]);
                if grp > 0 && !fault_is(6) {
                    b.cx(flag, g[grp - 1]);
                }
                return;
            }
            let len = t.row_len(hv);
            let mut inner = |b: &mut Builder, v: u64, f: Qubit| {
                let e = leaf_of(t, hv * row + v).expect("a visited value is a leaf");
                let (slot, grp, _) = place[e];
                b.cx(f, q[slot]);
                if grp > 0 && !fault_is(6) {
                    b.cx(f, g[grp - 1]);
                }
            };
            node_in_range(b, Some(flag), lo, 0, len, &mut inner);
        };
        node_in_range(b, None, hi, 0, t.rows(), &mut leaf);
    }
    let bb = t.spec.b as u64;
    let mut measured: Vec<Bit> = Vec::new();
    for (m, &f) in flags.iter().enumerate() {
        let base = fold.base[m];
        // (Mutant 23 shifts a class's slots by one.)
        let shift = usize::from(fault_is(23) && m == 0);
        let mut inner = |b: &mut Builder, v: u64, fl: Qubit| {
            b.cx(fl, q[(base + v as usize + shift) % e1]);
        };
        node_in_range(b, Some(f), lo, 0, bb, &mut inner);
        if early {
            measured.push(b.hmr(f));
        }
    }
    fold.flags = if early {
        measured
    } else {
        flags.iter().map(|&f| b.hmr(f)).collect()
    };
    Hot {
        q,
        g,
        e1,
        paired: false,
        fold: Some(fold),
        shared: false,
        sandwich: false,
        rec: None,
    }
}

/// The splits of an in-place unary expansion over `bits` index bits for the values `0..len`:
/// `(node, child, bit)` with `node`/`child` the leftmost values of their ranges; a one-sided node
/// passes through (values past `len` land where an in-range iteration sends them).
fn range_splits(bits: usize, len: u64) -> Vec<(usize, usize, usize)> {
    fn rec(out: &mut Vec<(usize, usize, usize)>, base: u64, k: usize, len: u64) {
        if k == 0 {
            return;
        }
        let half = 1u64 << (k - 1);
        if base + half < len {
            out.push((base as usize, (base + half) as usize, k - 1));
            rec(out, base, k - 1, len);
            rec(out, base + half, k - 1, len);
        } else {
            rec(out, base, k - 1, len);
        }
    }
    let mut out = Vec::new();
    rec(&mut out, 0, bits, len);
    out
}

/// A virtual row of lever `C`: row `hv`'s values whose top `s` lo bits equal `sub`, `len` of them
/// (the low `k_i - s` bits run over `0..len`).
#[derive(Clone, Copy, Debug)]
pub struct VRow {
    pub hv: usize,
    pub sub: u64,
    pub s: usize,
    pub len: u64,
}

/// Lever `C`'s virtual rows: each square row whole, each one-body row split by its top `s_ob`
/// lo bits (empty parts dropped).
#[must_use]
pub fn vrows(t: &SaTables<'_>, s_ob: usize) -> Vec<VRow> {
    let n_ob = t.rows() as usize - t.spec.r;
    vrows_by(t, &vec![s_ob; n_ob])
}

/// [`vrows`] with the square rows split too, by their top `s_sq` lo bits (lever `o`).
#[must_use]
pub fn vrows_sq(t: &SaTables<'_>, s_sq: usize, s_ob: usize) -> Vec<VRow> {
    let n_ob = t.rows() as usize - t.spec.r;
    vrows_rows(t, s_sq, &vec![s_ob; n_ob])
}

/// [`vrows`] with a split depth per one-body row (`depths[hv - R]`; lever `A`).
#[must_use]
pub fn vrows_by(t: &SaTables<'_>, depths: &[usize]) -> Vec<VRow> {
    vrows_rows(t, 0, depths)
}

/// Virtual rows: square rows split by their top `s_sq` lo bits, one-body row `hv` by its top
/// `depths[hv - R]` (levers `C`, `A`, `o`).
#[must_use]
pub fn vrows_rows(t: &SaTables<'_>, s_sq: usize, depths: &[usize]) -> Vec<VRow> {
    let mut out = Vec::new();
    for hv in 0..t.rows() as usize {
        let len = t.row_len(hv as u64);
        let s = if hv < t.spec.r {
            s_sq
        } else {
            depths[hv - t.spec.r]
        };
        let w = 1u64 << (t.k_i - s);
        for sub in 0..1u64 << s {
            let l = len.saturating_sub(sub * w).min(w);
            if l > 0 {
                out.push(VRow { hv, sub, s, len: l });
            }
        }
    }
    out
}

/// The classes of lever `C`: `G` consecutive virtual rows of one split depth each; returns
/// `(first, count, base slot, width)` per class and the slot count.
#[must_use]
pub fn classes_of(vr: &[VRow], groups: usize) -> (Vec<(usize, usize, usize, usize)>, usize) {
    let mut out = Vec::new();
    let mut at = 0;
    let mut i = 0;
    while i < vr.len() {
        let mut n = 1;
        while n < groups && i + n < vr.len() && vr[i + n].s == vr[i].s {
            n += 1;
        }
        let width = vr[i..i + n]
            .iter()
            .map(|v| v.len as usize)
            .max()
            .unwrap_or(0);
        out.push((i, n, at, width));
        at += width;
        i += n;
    }
    (out, at)
}

/// A class of lever `C`: its virtual rows (member `j` is group `j`; all of one split depth), its
/// first slot and its width.
#[derive(Clone, Debug)]
pub struct ClassDef {
    pub members: Vec<usize>,
    pub base: usize,
    pub width: usize,
}

/// Everything a lever-`C` write needs: the virtual rows, the classes, the slot count, and
/// (lever `.g`) the virtual rows grafted into other classes' spare cells.
#[derive(Clone, Debug)]
pub struct ClassPlan {
    pub vr: Vec<VRow>,
    pub defs: Vec<ClassDef>,
    pub e1: usize,
    pub grafts: Vec<Graft>,
}

/// Lever `.g`: virtual row `v` (a sub-row of a split one-body row) placed in class
/// `class`'s spare cells: group `group`, slots `base + off .. base + off + len`. Its lanes' `lo`
/// is XORed with `mask = (sub 2^(k_i - s)) ^ off` while the one-hot is live, so the class's own
/// expansion over `lo` lands them there (`off` is a multiple of `2^(k_i - s)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Graft {
    pub v: usize,
    pub class: usize,
    pub group: usize,
    pub off: usize,
    pub mask: u64,
}

/// Lever `.g` (`plan_grafts_inner`): grafts the last class's virtual rows into the spare cells of the
/// whole-row (depth 0) classes when every one of them fits, and drops that class. A square
/// member of a wider class keeps its identity stand-in at slot `len` (so cells from `len + 1`
/// are spare); a one-body member's cells from `len`; an empty group's from 0. Each graft's
/// offset is a multiple of its sub-row's span. Returns the grafts (empty when the last class is
/// not all sub-rows of split one-body rows, or something does not fit).
fn plan_grafts_inner(
    t: &SaTables<'_>,
    vr: &[VRow],
    defs: &mut Vec<ClassDef>,
    e1: &mut usize,
    groups: usize,
    prior: &[Graft],
) -> Vec<Graft> {
    let Some(last) = defs.last() else {
        return Vec::new();
    };
    if defs.len() < 2
        || last
            .members
            .iter()
            .any(|&v| vr[v].s == 0 || vr[v].hv < t.spec.r)
    {
        return Vec::new();
    }
    let n = defs.len() - 1;
    // Next free cell per (class, group), for the whole-row classes.
    let mut free: Vec<Vec<usize>> = defs[..n]
        .iter()
        .map(|d| {
            (0..groups)
                .map(|g| match d.members.get(g) {
                    _ if vr[d.members[0]].s != 0 => d.width,
                    Some(&m) => {
                        let len = vr[m].len as usize;
                        len + usize::from(vr[m].hv < t.spec.r && len < d.width)
                    }
                    None => 0,
                })
                .collect()
        })
        .collect();
    for gr in prior {
        let len = vr[gr.v].len as usize;
        let f = &mut free[gr.class][gr.group];
        *f = (*f).max(gr.off + len);
    }
    let mut out = Vec::new();
    for &v in &last.members {
        let span = 1usize << (t.k_i - vr[v].s);
        let len = vr[v].len as usize;
        let mut placed = None;
        'search: for (m, d) in defs[..n].iter().enumerate() {
            for (g, &fr) in free[m].iter().enumerate() {
                let off = fr.div_ceil(span) * span;
                if off + len <= d.width {
                    placed = Some((m, g, off));
                    break 'search;
                }
            }
        }
        let Some((m, g, off)) = placed else {
            return Vec::new();
        };
        free[m][g] = off + len;
        out.push(Graft {
            v,
            class: m,
            group: g,
            off,
            mask: (vr[v].sub * span as u64) ^ off as u64,
        });
    }
    let gone = defs.pop().expect("the last class");
    *e1 -= gone.width;
    out
}

/// Lever `o`: classes from the virtual rows of each split depth sorted by length
/// (longest first, stable), `G` at a time, so rows of similar length share a class.
fn defs_packed(vr: &[VRow], groups: usize) -> (Vec<ClassDef>, usize) {
    let mut depths: Vec<usize> = Vec::new();
    for v in vr {
        if !depths.contains(&v.s) {
            depths.push(v.s);
        }
    }
    let mut defs = Vec::new();
    let mut at = 0;
    for &d in &depths {
        let mut idx: Vec<usize> = (0..vr.len()).filter(|&i| vr[i].s == d).collect();
        idx.sort_by_key(|&i| std::cmp::Reverse(vr[i].len));
        for chunk in idx.chunks(groups) {
            let width = chunk.iter().map(|&i| vr[i].len as usize).max().unwrap_or(0);
            defs.push(ClassDef {
                members: chunk.to_vec(),
                base: at,
                width,
            });
            at += width;
        }
    }
    (defs, at)
}

/// [`defs_packed`] for the analysis tests.
#[must_use]
pub fn defs_packed_pub(vr: &[VRow], groups: usize) -> (Vec<ClassDef>, usize) {
    defs_packed(vr, groups)
}

/// Toffolis per copy that lever `C`'s write and erasure spend on `v` virtual rows of `rows` rows
/// in `c` classes over `e1` slots: the sub-row splits (`v - rows`), the class expansions
/// (`e1 - c`) and the unfold (`v - c`).
#[must_use]
pub fn class_toffolis(v: usize, rows: usize, c: usize, e1: usize) -> usize {
    2 * v + e1 - rows - 2 * c
}

/// Lever `o`'s exchange rate: one slot (a qubit on the SELECT plateau) is worth this many
/// Toffolis per copy, about half the step's `C / Q` on both est fronts (46 Li, 37 Reiher).
pub const PACK_TOFFOLIS_PER_SLOT: usize = 23;

/// Lever `o`: the split depths `(square, one-body)` minimising `23 e1 + (write and erasure
/// Toffolis per copy)`, then the fewest classes, then the shallowest.
#[must_use]
pub fn best_pack(t: &SaTables<'_>, groups: usize) -> (usize, usize) {
    #[cfg(test)]
    if let Some(p) = FORCE_PACK.with(std::cell::Cell::get) {
        return p;
    }
    let rows = t.rows() as usize;
    let mut best = None;
    for s_sq in 0..=t.k_i {
        for s_ob in 0..=t.k_i {
            let vr = vrows_sq(t, s_sq, s_ob);
            let (defs, e1) = defs_packed(&vr, groups);
            let cost = PACK_TOFFOLIS_PER_SLOT * e1 + class_toffolis(vr.len(), rows, defs.len(), e1);
            let key = (cost, defs.len(), s_sq + s_ob, s_sq);
            if best.as_ref().is_none_or(|(k, _)| key < *k) {
                best = Some((key, (s_sq, s_ob)));
            }
        }
    }
    best.map_or((0, 0), |(_, p)| p)
}

/// The class plan of lever `C` (consecutive classes, `best_s_ob`) or, with `packed`, of lever `o`.
#[must_use]
pub fn class_plan(t: &SaTables<'_>, groups: usize, packed: bool, mixed: bool) -> ClassPlan {
    if packed {
        let (s_sq, s_ob) = best_pack(t, groups);
        let vr = vrows_sq(t, s_sq, s_ob);
        let (defs, e1) = defs_packed(&vr, groups);
        return ClassPlan {
            vr,
            defs,
            e1,
            grafts: Vec::new(),
        };
    }
    let vr = vrows_by(t, &ob_depths(t, groups, mixed));
    let (cls, e1) = classes_of(&vr, groups);
    let defs = cls
        .iter()
        .map(|&(first, count, base, width)| ClassDef {
            members: (first..first + count).collect(),
            base,
            width,
        })
        .collect();
    ClassPlan {
        vr,
        defs,
        e1,
        grafts: Vec::new(),
    }
}

/// [`class_plan`] with lever `.g` ([`plan_grafts`]) when `graft` (not with `packed`).
#[must_use]
pub fn class_plan_g(
    t: &SaTables<'_>,
    groups: usize,
    packed: bool,
    mixed: bool,
    graft: bool,
) -> ClassPlan {
    let mut plan = class_plan(t, groups, packed, mixed);
    if !graft {
        return plan;
    }
    assert!(
        !packed,
        "lever .g grafts into lever C's consecutive classes, not o's"
    );
    // Every one-body split depth (as lever `A`'s search), grafting the trailing sub-row classes:
    // the fewest slots, then the fewest grafts, then the fewest write Toffolis.
    let n_ob = t.rows() as usize - t.spec.r;
    let radix = t.k_i + 1;
    let total = radix
        .checked_pow(n_ob as u32)
        .filter(|&x| x <= 1 << 16)
        .expect("lever .g: too many one-body rows for the depth search");
    let mut best: Option<((usize, usize, usize), ClassPlan)> = None;
    let mut d = vec![0usize; n_ob];
    #[cfg(test)]
    let forced = FORCE_GRAFT_DEPTHS.with(|c| c.borrow().clone());
    #[cfg(not(test))]
    let forced: Option<Vec<usize>> = None;
    for code in 0..total {
        let mut c = code;
        for x in &mut d {
            *x = c % radix;
            c /= radix;
        }
        if forced.as_ref().is_some_and(|f| *f != d) {
            continue;
        }
        let vr = vrows_by(t, &d);
        let (cls, mut e1) = classes_of(&vr, groups);
        let mut defs: Vec<ClassDef> = cls
            .iter()
            .map(|&(first, count, base, width)| ClassDef {
                members: (first..first + count).collect(),
                base,
                width,
            })
            .collect();
        let mut grafts = Vec::new();
        loop {
            let mut trial_defs = defs.clone();
            let mut trial_e1 = e1;
            let mut prior = grafts.clone();
            let g = plan_grafts_after(t, &vr, &mut trial_defs, &mut trial_e1, groups, &prior);
            if g.is_empty() {
                break;
            }
            prior.extend(g);
            grafts = prior;
            defs = trial_defs;
            e1 = trial_e1;
        }
        // A slot is a SELECT-plateau qubit (worth about half the step's C / Q per copy, as lever
        // `o`'s rate); a graft costs one Toffoli at the erasure and a live flag during the
        // write, a sub-row split one Toffoli each way.
        let subs = vr.len() - t.rows() as usize;
        let key = (
            PACK_TOFFOLIS_PER_SLOT * e1 + 2 * grafts.len() + 2 * subs,
            e1,
            grafts.len(),
        );
        if best.as_ref().is_none_or(|(k, _)| key < *k) {
            best = Some((
                key,
                ClassPlan {
                    vr,
                    defs,
                    e1,
                    grafts,
                },
            ));
        }
    }
    if let Some((k, p)) = best {
        if k.1 < plan.e1 || forced.is_some() {
            plan = p;
        }
    }
    plan
}

/// [`plan_grafts`] with the cells already taken by `prior` grafts left out.
fn plan_grafts_after(
    t: &SaTables<'_>,
    vr: &[VRow],
    defs: &mut Vec<ClassDef>,
    e1: &mut usize,
    groups: usize,
    prior: &[Graft],
) -> Vec<Graft> {
    plan_grafts_inner(t, vr, defs, e1, groups, prior)
}

/// The one-body split depth with the fewest slots (ties to the smaller depth).
#[must_use]
pub fn best_s_ob(t: &SaTables<'_>, groups: usize) -> usize {
    #[cfg(test)]
    if let Some(s) = FORCE_S_OB.with(std::cell::Cell::get) {
        return s.min(t.k_i);
    }
    (0..=t.k_i)
        .min_by_key(|&s| (classes_of(&vrows(t, s), groups).1, s))
        .unwrap_or(0)
}

/// Lever `A`: one split depth per one-body row, chosen jointly for the fewest
/// slots and then the fewest write and erase Toffolis. A one-body row left whole (depth 0) can
/// join the last square rows' partial class (its group would otherwise be empty), and the next
/// row can be cut fine enough to fill few slots. Without `A`, every one-body row takes the same
/// depth ([`best_s_ob`]), which is the layout of the earlier `C` bundles.
#[must_use]
pub fn ob_depths(t: &SaTables<'_>, groups: usize, mixed: bool) -> Vec<usize> {
    let n_ob = t.rows() as usize - t.spec.r;
    if !mixed {
        return vec![best_s_ob(t, groups); n_ob];
    }
    let cost = |d: &[usize]| {
        let vr = vrows_by(t, d);
        let (cls, e1) = classes_of(&vr, groups);
        // Sub-row splits, class expansions and the erasure's unfold ANDs.
        let subs = vr.len() - t.rows() as usize;
        let tof: usize = cls
            .iter()
            .map(|c| c.3.saturating_sub(1) + c.1 - 1)
            .sum::<usize>()
            + subs;
        (e1, tof)
    };
    let mut best = vec![0; n_ob];
    let mut best_cost = cost(&best);
    let mut d = vec![0; n_ob];
    let radix = t.k_i + 1;
    let total = radix.checked_pow(n_ob as u32).filter(|&x| x <= 1 << 16);
    let total = total.expect("lever A: too many one-body rows for the exhaustive depth search");
    for code in 0..total {
        let mut c = code;
        for x in &mut d {
            *x = c % radix;
            c /= radix;
        }
        let k = cost(&d);
        if k < best_cost {
            best_cost = k;
            best.clone_from(&d);
        }
    }
    best
}

/// Lever `C`'s layout ([`Fold`] for the correction tables): leaf `(row, lo)` in the virtual row
/// of its top lo bits, slot `base_m + (lo mod 2^(k_i - s))`, group = its place in the class.
#[must_use]
pub fn class_layout(t: &SaTables<'_>, groups: usize, mixed: bool) -> (Fold, usize) {
    class_layout_with(t, groups, false, mixed)
}

/// [`class_layout`], or with `packed` lever `o`'s layout ([`class_plan`]; `packed` takes
/// precedence over lever `A`'s `mixed`). A square row's identity lane (`lo = B`) is routed as the
/// write routes it: into the sub-row of its top bits, then by the in-range expansion to slot
/// `leaf_in_range(low, k_i - s, width)` of that class; that slot stands for the leaf
/// `L = leaf_in_range(B, k_i, B)` of the row (the checkpoint's identity fix assumes `L`), and if it
/// holds a real leaf, that leaf must be `L`.
#[must_use]
pub fn class_layout_with(
    t: &SaTables<'_>,
    groups: usize,
    packed: bool,
    mixed: bool,
) -> (Fold, usize) {
    if packed {
        return packed_layout(t, groups);
    }
    let n = leaves(t);
    let rows = t.rows() as usize;
    let row = 1u64 << t.k_i;
    let vr = vrows_by(t, &ob_depths(t, groups, mixed));
    let (cls, e1) = classes_of(&vr, groups);
    let mut place = vec![(0, 0, None); n];
    let mut leaf_at = vec![vec![None; e1]; groups];
    let mut class_of_row = vec![None; rows];
    for (m, &(first, count, base, width)) in cls.iter().enumerate() {
        for (g, v) in vr[first..first + count].iter().enumerate() {
            if v.sub == 0 {
                class_of_row[v.hv] = Some((m, g));
            }
            let w = 1u64 << (t.k_i - v.s);
            for low in 0..v.len {
                let lo = v.sub * w + low;
                let e = leaf_of(t, v.hv as u64 * row + lo).expect("a row value is a leaf");
                place[e] = (base + low as usize, g, Some(m));
                leaf_at[g][base + low as usize] = Some(e);
            }
            // A square row in a wider class: its identity item (`lo = B`) is in the class's range
            // and lands on slot `base + B`, which holds no leaf of this row. That slot stands for
            // the leaf an in-range iteration would have sent it to (same angles, and the same
            // fields for the checkpoint's identity fix).
            if v.hv < t.spec.r && (v.len as usize) < width {
                let lo2 = leaf_in_range(v.len, t.k_i, v.len);
                let e = leaf_of(t, v.hv as u64 * row + lo2).expect("an in-range leaf");
                leaf_at[g][base + v.len as usize] = Some(e);
            }
        }
    }
    let base = cls.iter().map(|c| c.2).collect();
    (
        Fold {
            class_of_row,
            base,
            place,
            leaf_at,
            flags: Vec::new(),
        },
        e1,
    )
}

/// [`class_layout_with`], or with lever `.g` the layout of [`class_plan_g`]: the
/// consecutive classes without the grafted class, and each grafted sub-row's leaves at
/// `(base + off + low, group)` of its host class.
#[must_use]
pub fn class_layout_g(
    t: &SaTables<'_>,
    groups: usize,
    packed: bool,
    mixed: bool,
    graft: bool,
) -> (Fold, usize) {
    if !graft {
        return class_layout_with(t, groups, packed, mixed);
    }
    let plan = class_plan_g(t, groups, packed, mixed, true);
    if plan.grafts.is_empty() {
        return class_layout_with(t, groups, packed, mixed);
    }
    let n = leaves(t);
    let rows = t.rows() as usize;
    let row = 1u64 << t.k_i;
    let vr = &plan.vr;
    let e1 = plan.e1;
    let mut place = vec![(0, 0, None); n];
    let mut leaf_at = vec![vec![None; e1]; groups];
    let mut class_of_row = vec![None; rows];
    for (m, d) in plan.defs.iter().enumerate() {
        for (g, &vi) in d.members.iter().enumerate() {
            let v = vr[vi];
            if v.sub == 0 {
                class_of_row[v.hv] = Some((m, g));
            }
            let w = 1u64 << (t.k_i - v.s);
            for low in 0..v.len {
                let lo = v.sub * w + low;
                let e = leaf_of(t, v.hv as u64 * row + lo).expect("a row value is a leaf");
                place[e] = (d.base + low as usize, g, Some(m));
                leaf_at[g][d.base + low as usize] = Some(e);
            }
            if v.hv < t.spec.r && (v.len as usize) < d.width {
                let lo2 = leaf_in_range(v.len, t.k_i, v.len);
                let e = leaf_of(t, v.hv as u64 * row + lo2).expect("an in-range leaf");
                leaf_at[g][d.base + v.len as usize] = Some(e);
            }
        }
    }
    for gr in &plan.grafts {
        let v = vr[gr.v];
        let d = &plan.defs[gr.class];
        let w = 1u64 << (t.k_i - v.s);
        for low in 0..v.len {
            let lo = v.sub * w + low;
            let e = leaf_of(t, v.hv as u64 * row + lo).expect("a row value is a leaf");
            let slot = d.base + gr.off + low as usize;
            assert!(
                leaf_at[gr.group][slot].is_none(),
                "a graft lands on a free cell"
            );
            place[e] = (slot, gr.group, Some(gr.class));
            leaf_at[gr.group][slot] = Some(e);
        }
    }
    let base = plan.defs.iter().map(|d| d.base).collect();
    (
        Fold {
            class_of_row,
            base,
            place,
            leaf_at,
            flags: Vec::new(),
        },
        e1,
    )
}

/// Lever `.g`: [`graft_mask`] of every leaf (all 0 without grafts).
#[must_use]
pub fn graft_masks(t: &SaTables<'_>, groups: usize, mixed: bool, graft: bool) -> Vec<u64> {
    let n = leaves(t);
    if !graft {
        return vec![0; n];
    }
    let plan = class_plan_g(t, groups, false, mixed, true);
    (0..n).map(|e| graft_mask(t, &plan, e)).collect()
}

/// Lever `.g`: the `lo` XOR mask of leaf `e` while the one-hot is live (0 unless `e` is
/// in a grafted sub-row).
#[must_use]
pub fn graft_mask(t: &SaTables<'_>, plan: &ClassPlan, e: usize) -> u64 {
    let (hv, lo, _) = leaf_fields(t, e);
    for gr in &plan.grafts {
        let v = plan.vr[gr.v];
        let w = 1u64 << (t.k_i - v.s);
        if v.hv as u64 == hv && lo / w == v.sub {
            return gr.mask;
        }
    }
    0
}

fn packed_layout(t: &SaTables<'_>, groups: usize) -> (Fold, usize) {
    let n = leaves(t);
    let rows = t.rows() as usize;
    let row = 1u64 << t.k_i;
    let plan = class_plan(t, groups, true, false);
    let e1 = plan.e1;
    let spec_b = t.spec.b as u64;
    let id_lo = leaf_in_range(spec_b, t.k_i, spec_b);
    let mut place = vec![(0, 0, None); n];
    let mut leaf_at = vec![vec![None; e1]; groups];
    let mut class_of_row = vec![None; rows];
    // Where each virtual row sits: (class, group).
    let mut at_of = vec![(0usize, 0usize); plan.vr.len()];
    for (m, d) in plan.defs.iter().enumerate() {
        for (g, &vi) in d.members.iter().enumerate() {
            at_of[vi] = (m, g);
            let v = plan.vr[vi];
            if v.sub == 0 {
                class_of_row[v.hv] = Some((m, g));
            }
            let w = 1u64 << (t.k_i - v.s);
            for low in 0..v.len {
                let lo = v.sub * w + low;
                let e = leaf_of(t, v.hv as u64 * row + lo).expect("a row value is a leaf");
                place[e] = (d.base + low as usize, g, Some(m));
                leaf_at[g][d.base + low as usize] = Some(e);
            }
        }
    }
    // Each square row's identity lane, routed as the write routes it: the sub-row split is an
    // in-range expansion over the row's `nsub` sub-rows, then the class expansion over its width.
    for hv in 0..t.spec.r {
        let subs: Vec<usize> = (0..plan.vr.len())
            .filter(|&i| plan.vr[i].hv == hv)
            .collect();
        let sd = plan.vr[subs[0]].s;
        let lbits = t.k_i - sd;
        let sub = leaf_in_range(spec_b >> lbits, sd, subs.len() as u64);
        let vi = subs[sub as usize];
        let (m, g) = at_of[vi];
        let d = &plan.defs[m];
        let x = leaf_in_range(spec_b & ((1 << lbits) - 1), lbits, d.width as u64);
        let slot = d.base + x as usize;
        let stand_in = if fault_is(81) { 0 } else { id_lo };
        let e = leaf_of(t, hv as u64 * row + stand_in).expect("the identity's stand-in leaf");
        match leaf_at[g][slot] {
            None => leaf_at[g][slot] = Some(e),
            Some(f) => assert!(
                f == e || fault_is(81),
                "an identity lane lands on a leaf other than L"
            ),
        }
    }
    let base = plan.defs.iter().map(|d| d.base).collect();
    (
        Fold {
            class_of_row,
            base,
            place,
            leaf_at,
            flags: Vec::new(),
        },
        e1,
    )
}

/// Lever `C`: writes the split one-hot in place. (1) The row one-hot of `hi` by
/// an in-place expansion (one AND per split; the root's first split is a copy), and each split
/// one-body row further by its top lo bits. (2) Each class folds its virtual rows: the class flag
/// `F_m` is their XOR (in the first one's qubit), the group bits collect them, and every other
/// one, equal to `F_m AND g_j` (the rows are exclusive), is measured with a `CZ(F_m, g_j)` fixup:
/// Clifford. (3) Each class expands its low lo bits in place under `F_m` (its root becomes slot
/// `base_m`). No class flag is left to measure; the erasure undoes the same steps
/// ([`erase_classes`]).
pub fn write_classes(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    groups: u8,
    mixed: bool,
) -> Hot {
    write_classes_with(b, t, lo, hi, groups, false, mixed)
}

/// [`write_classes`], or with `packed` lever `o`'s classes ([`class_plan`]): square rows split by
/// their top lo bits like the one-body rows, and the virtual rows of each depth grouped by length.
pub fn write_classes_with(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    groups: u8,
    packed: bool,
    mixed: bool,
) -> Hot {
    write_classes_pre(b, t, lo, hi, groups, packed, mixed, None, false)
}

/// [`write_classes_with`]; with `pre` (the unsplit one-hot only) every slot is the
/// caller's qubit `pre[slot]` (lever `E`'s register, allocated before the nested block and kept
/// allocated): the row, sub-row and expansion qubits are taken from it by the layout, which for
/// one group puts leaf `e` at slot `e`. `None` emits exactly [`write_classes_with`]'s ops.
///
/// # Panics
/// With `pre` and more than one group, or a layout whose slot is not its leaf.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn write_classes_pre(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    groups: u8,
    packed: bool,
    mixed: bool,
    pre: Option<&[Qubit]>,
    graft: bool,
) -> Hot {
    let groups = usize::from(groups);
    let (fold, e1) = class_layout_g(t, groups, packed, mixed, graft);
    let plan = class_plan_g(t, groups, packed, mixed, graft);
    assert!(
        plan.grafts.is_empty() || pre.is_none(),
        "lever .g needs the split one-hot"
    );
    let grafts = plan.grafts.clone();
    let vr = plan.vr;
    let rows = t.rows() as usize;
    // With `pre`: the slot each virtual row's root becomes (one class per virtual row).
    let base_of: Vec<usize> = if let Some(q) = pre {
        assert!(groups == 1, "a caller's register is the unsplit one-hot");
        assert!(
            fold.place.iter().enumerate().all(|(e, p)| p.0 == e) && q.len() == e1,
            "the unsplit class layout puts leaf e at slot e"
        );
        let mut base = vec![usize::MAX; vr.len()];
        for d in &plan.defs {
            base[d.members[0]] = d.base;
        }
        base
    } else {
        Vec::new()
    };
    let rq: Vec<Qubit> = if let Some(q) = pre {
        (0..rows)
            .map(|hv| {
                let v0 = vr
                    .iter()
                    .position(|x| x.hv == hv)
                    .expect("a row has a virtual row");
                q[base_of[v0]]
            })
            .collect()
    } else {
        b.alloc_n(rows)
    };
    let g = b.alloc_n(groups - 1);
    // (1) the row one-hot; the first split acts on the constant root: AND(1, hi_k) = hi_k.
    b.x(rq[0]);
    let row_splits = range_splits(hi.len(), rows as u64);
    for (i, &(node, child, k)) in row_splits.iter().enumerate() {
        if i == 0 {
            b.cx(hi[k], rq[child]);
        } else {
            b.ccx(rq[node], hi[k], rq[child]);
        }
        b.cx(rq[child], rq[node]);
    }
    // ... and the sub-rows of each split row (virtual row v's qubit vq[v]).
    let mut vq: Vec<Qubit> = Vec::with_capacity(vr.len());
    let mut row_v = vec![usize::MAX; rows];
    let mut sub_splits = Vec::new();
    let mut v = 0;
    while v < vr.len() {
        let hv = vr[v].hv;
        let nsub = vr[v..].iter().take_while(|x| x.hv == hv).count();
        row_v[hv] = v;
        vq.push(rq[hv]);
        for j in 1..nsub {
            vq.push(match pre {
                Some(q) => q[base_of[v + j]],
                None => b.alloc(),
            });
        }
        let s = vr[v].s;
        for (node, child, k) in range_splits(s, nsub as u64) {
            // lo bit (k_i - s + k) of the row's values.
            let bit = t.k_i - s + k;
            b.ccx(vq[v + node], lo[bit], vq[v + child]);
            b.cx(vq[v + child], vq[v + node]);
            sub_splits.push((v + node, v + child, bit));
        }
        v += nsub;
    }
    // (2) fold every class's virtual rows into F_m and the group bits.
    for d in &plan.defs {
        let m = &d.members;
        for j in 1..m.len() {
            b.cx(vq[m[j]], vq[m[0]]);
            b.cx(vq[m[j]], g[j - 1]);
        }
    }
    for d in &plan.defs {
        let m = &d.members;
        for j in 1..m.len() {
            let mm = b.hmr(vq[m[j]]);
            if !fault_is(42) {
                b.cz_if(vq[m[0]], g[j - 1], mm);
            }
        }
    }
    // Lever `.g`: each grafted sub-row joins its host class's flag and its group bit, and its
    // lanes' `lo` moves to the host slots (CNOTs from the sub-row's flag, kept until the host
    // class has expanded).
    for gr in &grafts {
        let host = vq[plan.defs[gr.class].members[0]];
        b.cx(vq[gr.v], host);
        if gr.group > 0 {
            b.cx(vq[gr.v], g[gr.group - 1]);
        }
        for (k, &x) in lo.iter().enumerate() {
            if gr.mask >> k & 1 == 1 && !(fault_is(120) && k == gr.mask.trailing_zeros() as usize) {
                b.cx(vq[gr.v], x);
            }
        }
    }
    // (3) each class expands its low lo bits under its flag. (Lever `.g`: the host classes
    // first, each followed by its grafts' measured erasure, so the graft flags are not live
    // while the other classes expand.)
    let mut q = vec![None; e1];
    let mut classes: Vec<Option<ClassSplits>> = vec![None; plan.defs.len()];
    let is_host = |m: usize| grafts.iter().any(|gr| gr.class == m);
    let order: Vec<usize> = (0..plan.defs.len())
        .filter(|&m| is_host(m))
        .chain((0..plan.defs.len()).filter(|&m| !is_host(m)))
        .collect();
    for m in order {
        let d = &plan.defs[m];
        let (first, base, width) = (d.members[0], d.base, d.width);
        let root = vq[first];
        q[base] = Some(root);
        let lbits = t.k_i - vr[first].s;
        let splits = range_splits(lbits, width as u64);
        for &(node, child, k) in &splits {
            let cq = match pre {
                Some(p) => p[base + child],
                None => b.alloc(),
            };
            q[base + child] = Some(cq);
            let nq = q[base + node].expect("a node exists before its child");
            b.ccx(nq, lo[k], cq);
            b.cx(cq, nq);
        }
        // Lever `.g`: a grafted sub-row's flag is now `H AND [group = g]` with `H` the parity of
        // its host slots (only its lanes reach them in group `g`), so it is X-measured and an
        // outcome of 1 is cancelled by `CZ(H, [group = g])`: `[group = 0] = 1 ^ XOR of the group
        // bits` (they are exclusive), so Cliffords only.
        for gr in grafts.iter().filter(|gr| gr.class == m) {
            let len = vr[gr.v].len as usize;
            let hs: Vec<Qubit> = (0..len)
                .map(|i| q[base + gr.off + i].expect("a host slot"))
                .collect();
            let mm = b.hmr(vq[gr.v]);
            if fault_is(121) {
                continue;
            }
            for &h in &hs {
                if gr.group > 0 {
                    b.cz_if(h, g[gr.group - 1], mm);
                } else {
                    b.z_if(h, mm);
                    for &x in &g {
                        b.cz_if(h, x, mm);
                    }
                }
            }
        }
        classes[m] = Some((d.members.clone(), base, splits));
    }
    let classes: Vec<ClassSplits> = classes
        .into_iter()
        .map(|c| c.expect("every class is expanded"))
        .collect();
    let q: Vec<Qubit> = q
        .into_iter()
        .map(|x| x.expect("every slot is written"))
        .collect();
    Hot {
        q,
        g,
        e1,
        paired: false,
        fold: Some(fold),
        shared: false,
        sandwich: false,
        rec: Some(std::rc::Rc::new(ClassRec {
            row_splits,
            sub_splits,
            classes,
            vq,
            row_v,
            keep: pre.is_some(),
            grafts: grafts
                .iter()
                .map(|gr| {
                    let d = &plan.defs[gr.class];
                    (*gr, d.base, vr[gr.v].len as usize)
                })
                .collect(),
        })),
    }
}

/// Lever `C`: erases a [`write_classes`] register exactly backwards. Each split's child is
/// `node AND bit` of qubits still present, so it is measured with a `CZ` fixup (Cliffords); only
/// the unfold, which recomputes a class's other virtual rows as `F_m AND g_j`, costs one Toffoli
/// per such row (`G - 1` per class). Needs `lo` and `hi` intact (the checkpoint rebuilds them).
pub fn erase_classes(b: &mut Builder, lo: &[Qubit], hi: &[Qubit], hot: Hot) {
    erase_classes_with(b, lo, hi, hot, false);
}

/// `.c`: the first sixteen rows are four classes named by the high two row bits, with four
/// groups named by the low two. Measure the flags and correct their phase as a degree-three
/// Boolean polynomial. The only charged term is an outcome-selected CCZ.
fn erase_factor_flags(b: &mut Builder, tail: Qubit, x: Qubit, y: Qubit, q: [Option<Qubit>; 4]) {
    use crate::circuit::{Op, OperationType};
    let ms: Vec<Bit> = q
        .into_iter()
        .map(|v| match v {
            Some(v) => b.hmr(v),
            None => super::narrow::zero_bit(b),
        })
        .collect();
    let parity = |b: &mut Builder, subset: &[usize]| {
        let p = super::narrow::zero_bit(b);
        for &j in subset {
            b.push_condition(ms[j]);
            let mut op = Op::new(OperationType::BitInvert);
            op.c_target = p.0;
            b.emit(op);
            b.pop_condition();
        }
        p
    };
    let p01 = parity(b, &[0, 1]);
    let p02 = parity(b, &[0, 2]);
    let pall = parity(b, &[0, 1, 2, 3]);
    b.x(tail); // first sixteen rows = NOT tail row
    b.z_if(tail, ms[0]);
    b.cz_if(tail, x, p01);
    b.cz_if(tail, y, p02);
    b.push_condition(pall);
    if !fault_is(220) {
        b.ccz(tail, x, y);
    }
    b.pop_condition();
    b.x(tail);
}

#[cfg(test)]
mod tests_factor_erase {
    use super::{erase_factor_flags, FAULT};
    use crate::circuit::Builder;
    use crate::walk::shared::testsim::Sim;

    fn lane(row: usize, seed: u64, fault: u8) -> bool {
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let hi = b.alloc_n(5);
        let cls = b.alloc_n(4);
        let grp = b.alloc_n(3);
        FAULT.with(|c| c.set(fault));
        erase_factor_flags(
            &mut b,
            hi[4],
            hi[2],
            hi[3],
            [Some(cls[0]), Some(cls[1]), Some(cls[2]), Some(cls[3])],
        );
        erase_factor_flags(
            &mut b,
            hi[4],
            hi[0],
            hi[1],
            [None, Some(grp[0]), Some(grp[1]), Some(grp[2])],
        );
        FAULT.with(|c| c.set(0));
        let mut sim = Sim::new(&b, seed);
        for (j, &q) in hi.iter().enumerate() {
            sim.set(q, row >> j & 1 == 1);
        }
        if row < 16 {
            sim.set(cls[row / 4], true);
            if row & 3 != 0 {
                sim.set(grp[row % 4 - 1], true);
            }
        }
        sim.run(b.ops());
        assert_eq!(sim.read(&hi), row as u64);
        for &q in &hi {
            sim.set(q, false);
        }
        std::panic::catch_unwind(|| sim.assert_clean()).is_ok()
    }

    #[test]
    fn every_row_and_measurement_seed_is_clean() {
        for row in 0..17 {
            for seed in 0..16 {
                assert!(lane(row, seed, 0), "row {row} seed {seed}");
            }
        }
    }

    #[test]
    fn missing_cubic_phase_is_detected() {
        assert!((0..16).any(|seed| !lane(15, seed, 220)));
    }
}

/// The ordinary class erasure, or `.c`'s factored phase correction for a rectangular
/// four-by-four first-row layout with a single grafted tail row.
pub fn erase_classes_with(b: &mut Builder, lo: &[Qubit], hi: &[Qubit], hot: Hot, factor: bool) {
    let rec = hot.rec.clone().expect("a write_classes register");
    let keep = rec.keep;
    let hmr = |b: &mut Builder, q: Qubit| {
        if keep {
            crate::walk::common::lookup::hmr_keep(b, q)
        } else {
            b.hmr(q)
        }
    };
    let mut vq = rec.vq.clone();
    // (Lever `.g`: the host classes last, so the recomputed graft flags are live only with the
    // host slots.)
    let host = |m: usize| rec.grafts.iter().any(|gr| gr.0.class == m);
    let order: Vec<usize> = (0..rec.classes.len())
        .rev()
        .filter(|&m| !host(m))
        .chain((0..rec.classes.len()).rev().filter(|&m| host(m)))
        .collect();
    for ci in order {
        let (members, base, splits) = &rec.classes[ci];
        // Lever `.g`: each sub-row grafted here is recomputed as `H AND [group = g]` (one Toffoli:
        // `H` gathered into its first host slot, `[group = 0]` into the first group bit, both in
        // place and undone) before the host class collapses.
        let mine: Vec<&(Graft, usize, usize)> =
            rec.grafts.iter().filter(|gr| gr.0.class == ci).collect();
        for &&(gr, gb, len) in &mine {
            let hs: Vec<Qubit> = (0..len).map(|i| hot.q[gb + gr.off + i]).collect();
            let gate = |b: &mut Builder| {
                for &h in &hs[1..] {
                    b.cx(h, hs[0]);
                }
                if gr.group == 0 {
                    for &x in &hot.g[1..] {
                        b.cx(x, hot.g[0]);
                    }
                    b.x(hot.g[0]);
                }
            };
            let gq = if gr.group == 0 {
                hot.g[0]
            } else {
                hot.g[gr.group - 1]
            };
            gate(b);
            let r = b.alloc();
            if !fault_is(122) {
                b.ccx(hs[0], gq, r);
            }
            // (undo the in-place gathers, in reverse)
            if gr.group == 0 {
                b.x(hot.g[0]);
                for &x in hot.g[1..].iter().rev() {
                    b.cx(x, hot.g[0]);
                }
            }
            for &h in hs[1..].iter().rev() {
                b.cx(h, hs[0]);
            }
            vq[gr.v] = r;
        }
        for &(node, child, k) in splits.iter().rev() {
            let (nq, cq) = (hot.q[base + node], hot.q[base + child]);
            b.cx(cq, nq);
            let m = hmr(b, cq);
            if !fault_is(43) {
                b.cz_if(nq, lo[k], m);
            }
        }
        // Lever `.g`: `lo` back, and the graft leaves the host flag and its group bit.
        for &&(gr, _, _) in &mine {
            for (k, &x) in lo.iter().enumerate() {
                if gr.mask >> k & 1 == 1 {
                    b.cx(vq[gr.v], x);
                }
            }
            b.cx(vq[gr.v], vq[members[0]]);
            if gr.group > 0 {
                b.cx(vq[gr.v], hot.g[gr.group - 1]);
            }
        }
    }
    if factor {
        assert!(
            !keep
                && hi.len() == 5
                && hot.g.len() == 3
                && rec.classes.len() == 4
                && rec.row_v.len() == 17
                && rec.grafts.len() == 3
                && rec.sub_splits.iter().all(|&(n, c, _)| n >= 16 && c >= 16)
                && rec.classes.iter().enumerate().all(|(i, (members, _, _))| {
                    members.len() == 4 && members.iter().enumerate().all(|(j, &v)| v == 4 * i + j)
                }),
            "lever .c needs four-by-four initial classes and one grafted tail row"
        );
        let cls: [Option<Qubit>; 4] = std::array::from_fn(|i| Some(vq[rec.classes[i].0[0]]));
        erase_factor_flags(b, hi[4], hi[2], hi[3], cls);
        let grp = [None, Some(hot.g[0]), Some(hot.g[1]), Some(hot.g[2])];
        erase_factor_flags(b, hi[4], hi[0], hi[1], grp);
        for &(node, child, bit) in rec.sub_splits.iter().rev() {
            b.cx(vq[child], vq[node]);
            let m = b.hmr(vq[child]);
            b.cz_if(vq[node], lo[bit], m);
        }
        let m = b.hmr(vq[rec.row_v[16]]);
        b.z_if(hi[4], m);
        return;
    }
    // Unfold: virtual row first + j = F_m AND g_j.
    let mut fresh = Vec::new();
    for (members, _, _) in &rec.classes {
        for j in 1..members.len() {
            let r = b.alloc();
            b.ccx(vq[members[0]], hot.g[j - 1], r);
            vq[members[j]] = r;
            fresh.push((members[0], j, r));
        }
    }
    for &(first, j, r) in &fresh {
        b.cx(r, hot.g[j - 1]);
        b.cx(r, vq[first]);
    }
    for &(node, child, bit) in rec.sub_splits.iter().rev() {
        b.cx(vq[child], vq[node]);
        let m = hmr(b, vq[child]);
        if !fault_is(44) {
            b.cz_if(vq[node], lo[bit], m);
        }
    }
    let row: Vec<Qubit> = rec.row_v.iter().map(|&v| vq[v]).collect();
    for (i, &(node, child, k)) in rec.row_splits.iter().enumerate().rev() {
        b.cx(row[child], row[node]);
        if i == 0 {
            b.cx(hi[k], row[child]);
            if keep {
                reset_keep(b, row[child]);
            } else {
                b.free(row[child]);
            }
            continue;
        }
        let m = hmr(b, row[child]);
        b.cz_if(row[node], hi[k], m);
    }
    b.x(row[0]);
    if keep {
        reset_keep(b, row[0]);
    } else {
        b.free(row[0]);
    }
    for &x in &hot.g {
        b.free(x);
    }
}

/// Unary iteration over `index` for values `base..limit` (from `base = 0`) that treats every lane
/// as in range: a node with only a left child passes its control down (`sa-pareto`'s
/// `iterate_in_range`).
fn node_in_range(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    base: u64,
    limit: u64,
    leaf: &mut dyn FnMut(&mut Builder, u64, Qubit),
) {
    let Some((&bit, low)) = index.split_last() else {
        leaf(b, base, ctl.expect("a leaf needs an indicator"));
        return;
    };
    let half = 1u64 << low.len();
    if base + half >= limit {
        node_in_range(b, ctl, low, base, limit, leaf);
        return;
    }
    let Some(c) = ctl else {
        b.x(bit);
        node_in_range(b, Some(bit), low, base, limit, leaf);
        b.x(bit);
        node_in_range(b, Some(bit), low, base + half, limit, leaf);
        return;
    };
    let child = b.alloc();
    b.x(bit);
    b.ccx(c, bit, child);
    b.x(bit);
    node_in_range(b, Some(child), low, base, limit, leaf);
    b.cx(c, child);
    node_in_range(b, Some(child), low, base + half, limit, leaf);
    erase_and(b, c, bit, child, false);
}

/// Moves the angle register from rotation `from`'s angle to rotation `to`'s (`None`: zero) by
/// CNOT fan-out from the one-hot. Cliffords only.
pub fn transition(
    b: &mut Builder,
    t: &SaTables<'_>,
    hot: &Hot,
    reg: &[Qubit],
    from: Option<usize>,
    to: Option<usize>,
) {
    transition_by(b, hot, reg, leaves(t), &|e, j| angle(t, e, j), from, to);
}

/// [`transition`] over any angle table `angle(leaf, rotation)` with `n_leaves` leaves (the
/// gadget the exhaustive tests drive directly).
pub fn transition_by(
    b: &mut Builder,
    hot: &Hot,
    reg: &[Qubit],
    n_leaves: usize,
    angle: &dyn Fn(usize, usize) -> u64,
    from: Option<usize>,
    to: Option<usize>,
) {
    let at = |i: usize, c: usize, j: usize| hot.leaf(i, c, n_leaves).map_or(0, |e| angle(e, j));
    let group_delta = |_e1: usize, i: usize, c: usize, j: usize| -> u64 {
        if hot.leaf(i, c, n_leaves).is_some() {
            at(0, c, j) ^ at(i, c, j)
        } else {
            0
        }
    };
    if from == to {
        return;
    }
    for (e, &h) in hot.q.iter().enumerate() {
        let a = from.map_or(0, |j| at(0, e, j));
        let c = to.map_or(0, |j| at(0, e, j));
        let d = a ^ c;
        for (k, &q) in reg.iter().enumerate() {
            if d >> k & 1 == 1 {
                b.cx(h, q);
            }
        }
    }
    if fault_is(5) {
        return;
    }
    // Group i's correction: bit k of the register gets g_i AND (D_i,from ^ D_i,to)_k(h).
    let deltas: Vec<Vec<u64>> = (0..hot.g.len())
        .map(|gi| {
            (0..hot.e1)
                .map(|c| {
                    from.map_or(0, |j| group_delta(hot.e1, gi + 1, c, j))
                        ^ to.map_or(0, |j| group_delta(hot.e1, gi + 1, c, j))
                })
                .collect()
        })
        .collect();
    let support = |d: &[u64], k: usize| -> Vec<Qubit> {
        hot.q
            .iter()
            .zip(d)
            .filter(|(_, &x)| x >> k & 1 == 1)
            .map(|(&h, _)| h)
            .collect()
    };
    let single = |b: &mut Builder, g: Qubit, set: &[Qubit], q: Qubit| match set {
        [] => {}
        [h] => b.ccx(g, *h, q),
        _ => {
            let s = b.alloc();
            set.iter().for_each(|&h| b.cx(h, s));
            b.ccx(g, s, q);
            set.iter().for_each(|&h| b.cx(h, s));
            b.free(s);
        }
    };
    // Lever `z`: group bits `a = g[2p]`, `c = g[2p + 1]` are never both 1 (the group bits are
    // one-hot), so `(a ^ X(h)) (c ^ Y(h)) = a Y(h) ^ c X(h) ^ X(h) Y(h)`. With `Y = D_a` and
    // `X = D_c` one Toffoli gives both groups' corrections of a register bit; the product
    // `X Y` is a function of the one-hot `h` alone, so its fan-out is CNOTs. The parities are
    // XORed into the group bits in place and undone (no scratch). An unpaired last group bit
    // keeps the single correction.
    let mut pairs = if hot.paired { hot.g.len() / 2 } else { 0 };
    // Lever `Z`: an odd group count >= 3 takes its last pair and the odd bit as a shared triple.
    let triple_at =
        (hot.shared && hot.paired && hot.g.len() >= 3 && hot.g.len() % 2 == 1).then(|| {
            pairs -= 1;
            2 * pairs
        });
    if let Some(t0) = triple_at {
        let ds = [&deltas[t0], &deltas[t0 + 1], &deltas[t0 + 2]];
        triple(
            b,
            &hot.q,
            [hot.g[t0], hot.g[t0 + 1], hot.g[t0 + 2]],
            ds,
            reg,
            hot.sandwich,
        );
    }
    for p in 0..pairs {
        let (a, c) = (hot.g[2 * p], hot.g[2 * p + 1]);
        let (da, dc) = (&deltas[2 * p], &deltas[2 * p + 1]);
        for (k, &q) in reg.iter().enumerate() {
            let y = support(da, k);
            let x = support(dc, k);
            match (x.is_empty(), y.is_empty()) {
                (true, true) => {}
                (true, false) => single(b, a, &y, q),
                (false, true) => single(b, c, &x, q),
                (false, false) => {
                    if !fault_is(20) {
                        for ((&h, &u), &v) in hot.q.iter().zip(da).zip(dc) {
                            if u >> k & 1 == 1 && v >> k & 1 == 1 {
                                b.cx(h, q);
                            }
                        }
                    }
                    let (xa, yc) = if fault_is(21) { (&y, &x) } else { (&x, &y) };
                    xa.iter().for_each(|&h| b.cx(h, a));
                    yc.iter().for_each(|&h| b.cx(h, c));
                    b.ccx(a, c, q);
                    xa.iter().for_each(|&h| b.cx(h, a));
                    yc.iter().for_each(|&h| b.cx(h, c));
                }
            }
        }
    }
    let skip = if triple_at.is_some() {
        hot.g.len()
    } else {
        2 * pairs
    };
    for (gi, &g) in hot.g.iter().enumerate().skip(skip) {
        for (k, &q) in reg.iter().enumerate() {
            single(b, g, &support(&deltas[gi], k), q);
        }
    }
}

/// Lever `Z`: the corrections of three exclusive group bits `[a, c, d]` with per-slot
/// differences `ds` (bit `k` of `ds[i][slot]` for group `i`), shared over pairs of register bits.
/// For bits `k1, k2` with targets `(A1, C1, D1)`, `(A2, C2, D2)` (vectors over the slots):
///
/// - `v1 = (a ^ d ^ C1)(c ^ A2)` gives `a: A2`, `c: C1`, `d: A2` and goes into both bits (one
///   scratch qubit, erased by measurement: its phase is the `CZ` of its two shifted controls);
/// - `v2 = (a ^ D1 ^ A2)(d ^ A1 ^ A2)` gives `a: A1 ^ A2`, `d: D1 ^ A2` (bit `k1`);
/// - `v3 = (c ^ D2 ^ A2)(d ^ C1 ^ C2)` gives `c: C1 ^ C2`, `d: D2 ^ A2` (bit `k2`).
///
/// Summed: bit `k1` gets `(A1, C1, D1)` and bit `k2` `(A2, C2, D2)` (group bits never two at a
/// time, so `(x.g ^ f)(y.g ^ f') = sum_l g_l (x_l f' ^ y_l f ^ x_l y_l) ^ f f'`); the products
/// `f f'` of the slot vectors are functions of the one-hot, cancelled by CNOT fan-out. A bit
/// left over (odd count) takes the pair `(a ^ D)(c ^ A)` plus the single `d`.
fn triple(
    b: &mut Builder,
    hq: &[Qubit],
    g: [Qubit; 3],
    ds: [&Vec<u64>; 3],
    reg: &[Qubit],
    sandwich: bool,
) {
    let n = hq.len();
    let vecbit =
        |d: &Vec<u64>, k: usize| -> Vec<bool> { (0..n).map(|c| d[c] >> k & 1 == 1).collect() };
    let xor =
        |x: &[bool], y: &[bool]| -> Vec<bool> { x.iter().zip(y).map(|(&p, &q)| p ^ q).collect() };
    let and =
        |x: &[bool], y: &[bool]| -> Vec<bool> { x.iter().zip(y).map(|(&p, &q)| p && q).collect() };
    let zero = |x: &[bool]| x.iter().all(|&v| !v);
    let fan = |b: &mut Builder, x: &[bool], t: Qubit| {
        for (c, &on) in x.iter().enumerate() {
            if on {
                b.cx(hq[c], t);
            }
        }
    };
    // `(u ^ f)(w ^ f')` into `target`, with the parities XORed into the controls in place, plus
    // the fan-out of `f f'` (unless fault 38 drops it).
    let prod = |b: &mut Builder, u: Qubit, f: &[bool], w: Qubit, f2: &[bool], target: Qubit| {
        fan(b, f, u);
        fan(b, f2, w);
        b.ccx(u, w, target);
        fan(b, f, u);
        fan(b, f2, w);
        if !fault_is(38) {
            fan(b, &and(f, f2), target);
        }
    };
    // Only bits that need both the pair and the single (two Toffolis alone) are shared; the
    // others keep `z`'s per-bit form, so the triple never costs more than `z`.
    let full = |k: usize| {
        let pair = !zero(&vecbit(ds[0], k)) || !zero(&vecbit(ds[1], k));
        pair && !zero(&vecbit(ds[2], k))
    };
    let mut ks: Vec<usize> = (0..reg.len()).filter(|&k| full(k)).collect();
    if ks.len() % 2 == 1 {
        ks.pop();
    }
    let rest: Vec<usize> = (0..reg.len()).filter(|k| !ks.contains(k)).collect();
    let [a, c, d] = g;
    let chunks: Vec<Vec<usize>> = ks
        .chunks(2)
        .map(<[usize]>::to_vec)
        .chain(rest.into_iter().map(|k| vec![k]))
        .collect();
    for chunk in &chunks {
        if let [k1, k2] = chunk[..] {
            let (a1, c1, d1) = (vecbit(ds[0], k1), vecbit(ds[1], k1), vecbit(ds[2], k1));
            let (a2, c2, d2) = (vecbit(ds[0], k2), vecbit(ds[1], k2), vecbit(ds[2], k2));
            // v1 = (a ^ d ^ C1)(c ^ A2) into both bits through a scratch qubit.
            b.cx(d, a);
            fan(b, &c1, a);
            fan(b, &a2, c);
            if sandwich {
                // Lever `Y`: `k2 ^= k1; k1 ^= v1; k2 ^= k1` puts `v1` into both bits.
                if !fault_is(49) {
                    b.cx(reg[k1], reg[k2]);
                }
                b.ccx(a, c, reg[k1]);
                b.cx(reg[k1], reg[k2]);
            } else {
                let t = b.alloc();
                b.ccx(a, c, t);
                b.cx(t, reg[k1]);
                if !fault_is(39) {
                    b.cx(t, reg[k2]);
                }
                let m = b.hmr(t);
                b.cz_if(a, c, m);
            }
            fan(b, &c1, a);
            fan(b, &a2, c);
            b.cx(d, a);
            let p1 = and(&c1, &a2);
            fan(b, &p1, reg[k1]);
            fan(b, &p1, reg[k2]);
            // v2 into k1, v3 into k2.
            prod(b, a, &xor(&d1, &a2), d, &xor(&a1, &a2), reg[k1]);
            prod(b, c, &xor(&d2, &a2), d, &xor(&c1, &c2), reg[k2]);
        } else {
            let k = chunk[0];
            let (a1, c1, d1) = (vecbit(ds[0], k), vecbit(ds[1], k), vecbit(ds[2], k));
            // (a ^ C)(c ^ A) gives a: A, c: C; the single d: D.
            if !(zero(&a1) && zero(&c1)) {
                prod(b, a, &c1, c, &a1, reg[k]);
            }
            let sup: Vec<Qubit> = (0..n).filter(|&s| d1[s]).map(|s| hq[s]).collect();
            match sup.as_slice() {
                [] => {}
                [h] => b.ccx(d, *h, reg[k]),
                _ => {
                    let sq = b.alloc();
                    sup.iter().for_each(|&h| b.cx(h, sq));
                    b.ccx(d, sq, reg[k]);
                    sup.iter().for_each(|&h| b.cx(h, sq));
                    b.free(sq);
                }
            }
        }
    }
}

/// Lever `a`: unloads the angle register by X measurement instead of a
/// transition to zero. Outcome `m_k` of bit `k` leaves `(-1)^(m_k A_k(g, h))` with `A = angle(.,
/// cur)` as the one-hot holds it: `L_k(h) ^ sum_i g_i D_ik(h)`, or with lever `z` the paired form.
/// Every part is a Clifford phase: `Z` on the slots of `L_k`, `CZ(g_i, parity)` for a group term,
/// and for a pair `CZ(a ^ X, c ^ Y)` (parities XORed into the group bits and undone) plus `Z` on the
/// slots of `X Y`; each conditioned on `m_k`. 0 Toffolis. The register stays allocated (`|0>`).
pub fn unload_measured(
    b: &mut Builder,
    hot: &Hot,
    reg: &[Qubit],
    n_leaves: usize,
    angle: &dyn Fn(usize, usize) -> u64,
    cur: usize,
) {
    let m: Vec<Bit> = reg
        .iter()
        .map(|&q| crate::walk::common::lookup::hmr_keep(b, q))
        .collect();
    let at = |i: usize, c: usize| hot.leaf(i, c, n_leaves).map_or(0, |e| angle(e, cur));
    let delta = |i: usize, c: usize| {
        if hot.leaf(i, c, n_leaves).is_some() {
            at(0, c) ^ at(i, c)
        } else {
            0
        }
    };
    let deltas: Vec<Vec<u64>> = (1..=hot.g.len())
        .map(|i| (0..hot.e1).map(|c| delta(i, c)).collect())
        .collect();
    let support = |d: &[u64], k: usize| -> Vec<Qubit> {
        hot.q
            .iter()
            .zip(d)
            .filter(|(_, &x)| x >> k & 1 == 1)
            .map(|(&h, _)| h)
            .collect()
    };
    let pairs = if hot.paired { hot.g.len() / 2 } else { 0 };
    for (k, &mk) in m.iter().enumerate() {
        if fault_is(24) && k == 0 {
            continue;
        }
        b.push_condition(mk);
        for (c, &h) in hot.q.iter().enumerate() {
            if at(0, c) >> k & 1 == 1 {
                b.z(h);
            }
        }
        for p in 0..pairs {
            let (a, cq) = (hot.g[2 * p], hot.g[2 * p + 1]);
            let y = support(&deltas[2 * p], k);
            let x = support(&deltas[2 * p + 1], k);
            for ((&h, &u), &v) in hot.q.iter().zip(&deltas[2 * p]).zip(&deltas[2 * p + 1]) {
                if u >> k & 1 == 1 && v >> k & 1 == 1 {
                    b.z(h);
                }
            }
            x.iter().for_each(|&h| b.cx(h, a));
            y.iter().for_each(|&h| b.cx(h, cq));
            b.cz(a, cq);
            x.iter().for_each(|&h| b.cx(h, a));
            y.iter().for_each(|&h| b.cx(h, cq));
        }
        for (gi, &g) in hot.g.iter().enumerate().skip(2 * pairs) {
            // (-1)^(g P(h)) with P a parity of slots: CZ(g, h_c) for each slot c of P.
            for h in support(&deltas[gi], k) {
                b.cz(g, h);
            }
        }
        b.pop_condition();
    }
    for &q in reg {
        reset_keep(b, q);
    }
}

/// The network index fields a leaf was written from: `(row, lo, is_ob)`, the inverse of
/// [`leaf_of`] (a square leaf `e < R B` is row `e / B`, `lo = e mod B`; a one-body leaf `R B + r`
/// is row `R + r / 2^k_i`, `lo = r mod 2^k_i`).
#[must_use]
pub fn leaf_fields(t: &SaTables<'_>, e: usize) -> (u64, u64, u64) {
    let s = t.spec;
    if e < s.r * s.b {
        ((e / s.b) as u64, (e % s.b) as u64, 0)
    } else {
        let r = e - s.r * s.b;
        (
            (s.r + (r >> t.k_i)) as u64,
            (r & ((1 << t.k_i) - 1)) as u64,
            1,
        )
    }
}

/// `R` on a qubit the builder keeps allocated (it must be `|0>`): ends its lifetime for the
/// harness's liveness count without releasing it.
pub fn reset_keep(b: &mut Builder, q: Qubit) {
    let mut op = Op::new(OperationType::R);
    op.q_target = q.0;
    b.emit(op);
}

/// Erases the one-hot register (freeing it): X-basis measurements, then one phase fixup over the
/// padded network index `idx` (`limit` values) split as `split`, with the leaf map the write used.
pub fn erase(
    b: &mut Builder,
    t: &SaTables<'_>,
    idx: &[Qubit],
    limit: u64,
    hot: Hot,
    split: Split,
    in_range: bool,
) {
    erase_keep(b, t, idx, limit, hot, split, in_range, false);
}

/// [`erase`]; with `keep` (lever `E`) the measured qubits stay allocated (`|0>`, not live).
#[allow(clippy::too_many_arguments)]
pub fn erase_keep(
    b: &mut Builder,
    t: &SaTables<'_>,
    idx: &[Qubit],
    limit: u64,
    hot: Hot,
    split: Split,
    in_range: bool,
    keep: bool,
) {
    let bits = hot.q.len() + hot.g.len();
    let mut m: Vec<Bit> = hot
        .q
        .iter()
        .chain(&hot.g)
        .map(|&q| {
            if keep {
                crate::walk::common::lookup::hmr_keep(b, q)
            } else {
                b.hmr(q)
            }
        })
        .collect();
    if fault_is(1) {
        return;
    }
    let nflags = hot.fold.as_ref().map_or(0, |f| f.flags.len());
    if let Some(f) = &hot.fold {
        if !fault_is(22) {
            m.extend(f.flags.iter().copied());
        }
    }
    let nflags = if fault_is(22) { 0 } else { nflags };
    let shift = u64::from(fault_is(2));
    let data = |v: u64| {
        let leaf = lane_leaf(t, v + shift, in_range);
        if hot.g.is_empty() {
            return unit(bits, leaf);
        }
        let mut w = word(bits + nflags);
        if let Some(e) = leaf {
            let (slot, group) = hot.place(e);
            w[slot / 64] |= 1 << (slot % 64);
            if group > 0 {
                let at = hot.e1 + group - 1;
                w[at / 64] |= 1 << (at % 64);
            }
            if let Some(f) = hot.fold.as_ref().filter(|_| nflags > 0) {
                if let Some(c) = f.place[e].2 {
                    let at = bits + c;
                    w[at / 64] |= 1 << (at % 64);
                }
            }
        }
        w
    };
    qroam::fixup(b, idx, limit, &m, &data, split);
}

/// XORs `value(e)` into `targets` (bit `k` of the value into `targets[k]`) for the lane's leaf
/// `e`, from the one-hot (lever `C`): CNOTs from each slot for its group-0 leaf, and for a split
/// one-hot the group corrections of [`transition_by`] (one Toffoli per group bit and target bit
/// whose group difference is not zero, or per pair of group bits with lever `z`). Nothing on a
/// lane whose one-hot is empty.
pub fn fan(
    b: &mut Builder,
    t: &SaTables<'_>,
    hot: &Hot,
    targets: &[Qubit],
    value: &dyn Fn(usize) -> u64,
) {
    // A fan-out is a transition from nothing to a one-rotation table: the split one-hot's group
    // corrections (paired with lever `z`) come for free from `transition_by`.
    transition_by(b, hot, targets, leaves(t), &|e, _| value(e), None, Some(0));
}

/// The outer-item and network-index fields a leaf was written from (lever `C`): `lo | row <<
/// k_i | is_ob << (k_i + h) | pos_e << (k_i + h + 1)`. A square leaf `e < R B` is row `e / B`,
/// `lo = e mod B`, `is_ob = pos_e = 0`; a one-body leaf `R B + r` is row `R + r / 2^k_i`, `lo = r
/// mod 2^k_i`, `is_ob = 1`, `pos_e = [e_r >= 0]` (`SaTables::item_flags`).
#[must_use]
pub fn leaf_index_value(t: &SaTables<'_>, e: usize) -> u64 {
    let s = t.spec;
    let (k_i, h) = (t.k_i, t.h);
    if e < s.r * s.b {
        ((e % s.b) as u64) | ((e / s.b) as u64) << k_i
    } else {
        let r = e - s.r * s.b;
        let row = (s.r + (r >> k_i)) as u64;
        let lo = (r & ((1 << k_i) - 1)) as u64;
        lo | row << k_i | 1 << (k_i + h) | u64::from(s.e[r] >= 0.0) << (k_i + h + 1)
    }
}

/// Lever `E`: where rotation `j`'s angle register lives inside an unsplit one-hot. Over the
/// leaves, the register's bits are `w_j` linear functions of the one-hot; `r` of them (the rank)
/// are independent, and `leaves` are `r` leaves on which those bits form an invertible matrix.
/// Their one-hot qubits become the register's independent bits: `ops` are CNOTs `(control,
/// target)` between them (indices into `leaves`) that turn their one-hot values `x` into
/// `Q^-1 x` (column Gauss-Jordan of `A[k][l] = bit k of angle(leaves[l])`, `A Q = P`), after which
/// bit `k` is on qubit `bit[k]`. A dependent bit (`bit[k] = None`, only on toy specs with fewer
/// leaves than angle bits) gets its own qubit, filled by the plain fan-out.
#[derive(Clone, Debug)]
pub struct Pivot {
    pub leaves: Vec<usize>,
    pub ops: Vec<(usize, usize)>,
    pub bit: Vec<Option<usize>>,
}

impl Pivot {
    /// Register bits that need a qubit of their own.
    #[must_use]
    pub fn extras(&self) -> Vec<usize> {
        (0..self.bit.len())
            .filter(|&k| self.bit[k].is_none())
            .collect()
    }
}

/// The [`Pivot`] of rotation `j` over an angle table with `n` leaves.
#[must_use]
pub fn pivot_by(n: usize, w: usize, angle: &dyn Fn(usize) -> u64) -> Pivot {
    let words = n.div_ceil(64);
    let row = |k: usize| -> Vec<u64> {
        let mut v = vec![0u64; words];
        for e in 0..n {
            if angle(e) >> k & 1 == 1 {
                v[e / 64] |= 1 << (e % 64);
            }
        }
        v
    };
    // Row echelon over the bits: basis rows with their pivot leaf.
    let mut basis: Vec<(Vec<u64>, usize)> = Vec::new();
    let mut chosen: Vec<usize> = Vec::new();
    for k in 0..w {
        let mut v = row(k);
        for (bv, pl) in &basis {
            if v[pl / 64] >> (pl % 64) & 1 == 1 {
                for (x, y) in v.iter_mut().zip(bv) {
                    *x ^= y;
                }
            }
        }
        if let Some(pl) = (0..n).find(|&e| v[e / 64] >> (e % 64) & 1 == 1) {
            basis.push((v, pl));
            chosen.push(k);
        }
    }
    let leaves: Vec<usize> = basis.iter().map(|&(_, pl)| pl).collect();
    let r = leaves.len();
    // Column l restricted to the chosen bits (bit i of the vector = chosen bit chosen[i]).
    let mut cols: Vec<u64> = leaves
        .iter()
        .map(|&e| {
            let a = angle(e);
            chosen
                .iter()
                .enumerate()
                .fold(0, |acc, (i, &k)| acc | (a >> k & 1) << i)
        })
        .collect();
    let mut used = vec![false; r];
    let mut bit = vec![None; w];
    let mut ops = Vec::new();
    for (i, &k) in chosen.iter().enumerate() {
        let c = (0..r)
            .find(|&c| !used[c] && cols[c] >> i & 1 == 1)
            .expect("the chosen bits are invertible on the chosen leaves");
        used[c] = true;
        bit[k] = Some(c);
        for c2 in 0..r {
            if c2 != c && cols[c2] >> i & 1 == 1 {
                cols[c2] ^= cols[c];
                ops.push((c2, c));
            }
        }
    }
    Pivot { leaves, ops, bit }
}

/// [`pivot_by`] for rotation `j` of the tables.
#[must_use]
pub fn pivot(t: &SaTables<'_>, j: usize) -> Pivot {
    pivot_by(leaves(t), t.widths[j], &|e| angle(t, e, j))
}

/// The qubits of rotation `j`'s embedded register, bit `k` first (lever `E`); a dependent bit
/// takes the next qubit of `extra`.
#[must_use]
pub fn pivot_register(hot_q: &[Qubit], extra: &[Qubit], pv: &Pivot) -> Vec<Qubit> {
    let mut x = extra.iter();
    pv.bit
        .iter()
        .map(|b| match b {
            Some(l) => hot_q[pv.leaves[*l]],
            None => *x.next().expect("enough extra qubits"),
        })
        .collect()
}

/// Lever `E`: loads rotation `j`'s angle into its embedded register (`load`) or returns the
/// register to the plain one-hot (`!load`), Cliffords only. Loaded, the qubit of bit `k` holds
/// `sum_e h[e] (bit k of angle(e))`; every other one-hot qubit is unchanged.
pub fn embed_by(
    b: &mut Builder,
    hot_q: &[Qubit],
    extra: &[Qubit],
    pv: &Pivot,
    angle: &dyn Fn(usize) -> u64,
    load: bool,
) {
    let reg = pivot_register(hot_q, extra, pv);
    let q = |l: usize| hot_q[pv.leaves[l]];
    let extras = |b: &mut Builder| {
        for k in pv.extras() {
            for (e, &h) in hot_q.iter().enumerate() {
                if angle(e) >> k & 1 == 1 {
                    b.cx(h, reg[k]);
                }
            }
        }
    };
    let rest = |b: &mut Builder| {
        if fault_is(14) {
            return;
        }
        for (e, &h) in hot_q.iter().enumerate() {
            if pv.leaves.contains(&e) {
                continue;
            }
            let a = angle(e);
            for (k, slot) in pv.bit.iter().enumerate() {
                if slot.is_some() && a >> k & 1 == 1 {
                    b.cx(h, reg[k]);
                }
            }
        }
    };
    if load {
        extras(b);
        let skip = usize::from(fault_is(19));
        for &(c, tg) in pv.ops.iter().skip(skip) {
            b.cx(q(c), q(tg));
        }
        rest(b);
    } else {
        rest(b);
        let skip = usize::from(fault_is(19));
        for &(c, tg) in pv.ops.iter().skip(skip).rev() {
            b.cx(q(c), q(tg));
        }
        extras(b);
    }
}

/// [`embed_by`] for rotation `j` of the tables.
pub fn embed(
    b: &mut Builder,
    t: &SaTables<'_>,
    hot: &Hot,
    extra: &[Qubit],
    pv: &Pivot,
    j: usize,
    load: bool,
) {
    assert!(hot.g.is_empty(), "lever E needs the unsplit one-hot");
    embed_by(b, &hot.q, extra, pv, &|e| angle(t, e, j), load);
}

#[cfg(test)]
mod embed_tests {
    //! Exhaustive gadget test of lever `E`: for random angle tables (more and fewer leaves than
    //! angle bits), every leaf, every rotation, the embedded register holds the leaf's angle after
    //! the load and the one-hot is plain again after the unload; mutants 14 (non-pivot leaves
    //! left out) and 19 (one Gauss-Jordan CNOT dropped) leave a wrong angle.
    use super::{embed_by, pivot_by, pivot_register, FAULT};
    use crate::circuit::Builder;
    use crate::walk::shared::testsim::Sim;

    fn table(seed: u64, n: usize, rots: usize, bits: usize) -> Vec<Vec<u64>> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..n)
            .map(|_| {
                (0..rots)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x % (1 << bits)
                    })
                    .collect()
            })
            .collect()
    }

    /// Loads and unloads every rotation for leaf `e`; returns the register values seen.
    fn run(tab: &[Vec<u64>], bits: usize, e: usize, fault: u8) -> Vec<u64> {
        let n = tab.len();
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let hot = b.alloc_n(n);
        let pivs: Vec<_> = (0..tab[0].len())
            .map(|j| pivot_by(n, bits, &|l| tab[l][j]))
            .collect();
        let nx = pivs.iter().map(|p| p.extras().len()).max().unwrap_or(0);
        let extra = b.alloc_n(nx);
        let mut sim = Sim::new(&b, 3);
        sim.set(hot[e], true);
        let mut seen = Vec::new();
        FAULT.with(|f| f.set(fault));
        for (j, pv) in pivs.iter().enumerate() {
            let start = b.ops().len();
            embed_by(&mut b, &hot, &extra, pv, &|l| tab[l][j], true);
            sim.run(&b.ops()[start..]);
            seen.push(sim.read(&pivot_register(&hot, &extra, pv)));
            let start = b.ops().len();
            embed_by(&mut b, &hot, &extra, pv, &|l| tab[l][j], false);
            sim.run(&b.ops()[start..]);
        }
        FAULT.with(|f| f.set(0));
        let plain: Vec<u64> = hot.iter().map(|&q| sim.read(&[q])).collect();
        let want: Vec<u64> = (0..n).map(|l| u64::from(l == e)).collect();
        assert_eq!(plain, want, "the one-hot is not plain after the unloads");
        assert_eq!(sim.read(&extra), 0, "extras left dirty");
        sim.set(hot[e], false);
        sim.assert_clean();
        seen
    }

    #[test]
    fn embedded_register_holds_every_angle() {
        for (n, bits, seed) in [
            (5usize, 8usize, 1u64),
            (9, 6, 2),
            (24, 8, 3),
            (40, 16, 4),
            (13, 13, 5),
        ] {
            let tab = table(seed, n, 5, bits);
            for e in 0..n {
                assert_eq!(run(&tab, bits, e, 0), tab[e], "n {n} bits {bits} leaf {e}");
            }
        }
    }

    #[test]
    fn embed_mutants_leave_a_wrong_angle() {
        for (n, bits, seed) in [(24usize, 8usize, 3u64), (40, 16, 4)] {
            let tab = table(seed, n, 5, bits);
            for fault in [14u8, 19] {
                let wrong = (0..n).any(|e| {
                    std::panic::catch_unwind(|| run(&tab, bits, e, fault))
                        .map_or(true, |seen| seen != tab[e])
                });
                assert!(wrong, "n {n}: fault {fault} not caught");
            }
        }
    }
}

#[cfg(test)]
mod paired_tests {
    //! Exhaustive gadget tests of the split one-hot's transitions, unpaired and paired (lever
    //! `z`): for every leaf of small random angle tables, the register holds each rotation's angle
    //! after its transition and is `|0>` at the end, the paired form costs `ceil((G - 1) / 2)`
    //! Toffolis per corrected bit, and its mutants (faults 20, 21) leave a wrong angle.
    use super::{transition_by, Hot, FAULT};
    use crate::circuit::Builder;
    use crate::walk::shared::testsim::Sim;

    fn table(seed: u64, leaves: usize, rots: usize, bits: usize) -> Vec<Vec<u64>> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..leaves)
            .map(|_| {
                (0..rots)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x % (1 << bits)
                    })
                    .collect()
            })
            .collect()
    }

    /// Runs the chain `None -> 0 -> .. -> J-1 -> None` for leaf `e`; returns the register after
    /// each load (and the Toffolis) or panics if the end is not clean.
    fn chain(
        n: usize,
        groups: usize,
        paired: bool,
        tab: &[Vec<u64>],
        bits: usize,
        e: usize,
        fault: u8,
    ) -> (Vec<u64>, u64) {
        let rots = tab[0].len();
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let e1 = n.div_ceil(groups);
        let hot = Hot {
            q: b.alloc_n(e1),
            g: b.alloc_n(groups - 1),
            e1,
            paired,
            fold: None,
            shared: false,
            sandwich: false,
            rec: None,
        };
        let reg = b.alloc_n(bits);
        let angle = |l: usize, j: usize| tab[l][j];
        let mut seen = Vec::new();
        let mut sim = Sim::new(&b, 7);
        sim.set(hot.q[e % e1], true);
        if e / e1 > 0 {
            sim.set(hot.g[e / e1 - 1], true);
        }
        let mut cur = None;
        FAULT.with(|c| c.set(fault));
        for j in (0..rots).map(Some).chain([None]) {
            let start = b.ops().len();
            transition_by(&mut b, &hot, &reg, n, &angle, cur, j);
            sim.run(&b.ops()[start..]);
            if j.is_some() {
                seen.push(sim.read(&reg));
            }
            cur = j;
        }
        FAULT.with(|c| c.set(0));
        assert_eq!(sim.read(&reg), 0, "register not unloaded");
        sim.set(hot.q[e % e1], false);
        if e / e1 > 0 {
            sim.set(hot.g[e / e1 - 1], false);
        }
        sim.assert_clean();
        (seen, sim.toffolis)
    }

    #[test]
    fn paired_transitions_are_exact_for_every_leaf() {
        let bits = 6;
        for n in [5usize, 7, 9, 10, 13] {
            for groups in 2..=7usize.min(n) {
                let tab = table((n * 31 + groups) as u64, n, 4, bits);
                let (mut t_plain, mut t_paired) = (0, 0);
                for e in 0..n {
                    let want: Vec<u64> = tab[e].clone();
                    let (a, ta) = chain(n, groups, false, &tab, bits, e, 0);
                    let (c, tc) = chain(n, groups, true, &tab, bits, e, 0);
                    assert_eq!(a, want, "unpaired n {n} G {groups} leaf {e}");
                    assert_eq!(c, want, "paired n {n} G {groups} leaf {e}");
                    // Toffolis do not depend on the lane.
                    t_plain = ta;
                    t_paired = tc;
                }
                // Every pair of group bits costs at most one Toffoli per register bit and
                // transition, where the unpaired form costs one per group bit.
                assert!(t_paired <= t_plain, "n {n} G {groups}");
                let pairs = (groups - 1) / 2;
                let slots = (rots_plus_one(&tab)) * bits;
                assert!(
                    t_paired <= ((groups - 1).div_ceil(2) * slots) as u64,
                    "n {n} G {groups}: {t_paired}"
                );
                if pairs > 0 && n >= 7 {
                    assert!(
                        t_paired < t_plain,
                        "n {n} G {groups}: pairing saved nothing"
                    );
                }
            }
        }
    }

    /// Lever `a`: for every leaf, rotation and outcome seed, loading the angle and unloading it
    /// by measurement leaves every ancilla `|0>` and phase `+1`, with 0 Toffolis in the unload;
    /// mutant 24 leaves phase garbage on some lane.
    #[test]
    fn measured_unload_is_clean_for_every_leaf() {
        use super::unload_measured;
        let bits = 6;
        for n in [5usize, 9, 10, 13] {
            for groups in 1..=5usize.min(n) {
                for paired in [false, true] {
                    let tab = table((n * 7 + groups) as u64, n, 3, bits);
                    for fault in [0u8, 24] {
                        let mut caught = false;
                        for e in 0..n {
                            for j in 0..3 {
                                for seed in 1..5u64 {
                                    let mut b = Builder::new(1);
                                    b.declare_uniform(1);
                                    let e1 = n.div_ceil(groups);
                                    let hot = Hot {
                                        q: b.alloc_n(e1),
                                        g: b.alloc_n(groups - 1),
                                        e1,
                                        paired,
                                        fold: None,
                                        shared: false,
                                        sandwich: false,
                                        rec: None,
                                    };
                                    let reg = b.alloc_n(bits);
                                    let angle = |l: usize, r: usize| tab[l][r];
                                    let mut sim = Sim::new(&b, seed * 977 + e as u64);
                                    sim.set(hot.q[e % e1], true);
                                    if e / e1 > 0 {
                                        sim.set(hot.g[e / e1 - 1], true);
                                    }
                                    transition_by(&mut b, &hot, &reg, n, &angle, None, Some(j));
                                    let mid = b.ops().len();
                                    FAULT.with(|c| c.set(fault));
                                    unload_measured(&mut b, &hot, &reg, n, &angle, j);
                                    FAULT.with(|c| c.set(0));
                                    sim.run(b.ops());
                                    assert_eq!(sim.read(&reg), 0);
                                    let tof = b.ops()[mid..]
                                        .iter()
                                        .filter(|o| {
                                            matches!(
                                                o.kind,
                                                crate::circuit::OperationType::CCX
                                                    | crate::circuit::OperationType::CCZ
                                            )
                                        })
                                        .count();
                                    assert_eq!(tof, 0, "the measured unload is Clifford");
                                    sim.set(hot.q[e % e1], false);
                                    if e / e1 > 0 {
                                        sim.set(hot.g[e / e1 - 1], false);
                                    }
                                    if fault == 0 {
                                        sim.assert_clean();
                                    } else if std::panic::catch_unwind(|| sim.assert_clean())
                                        .is_err()
                                    {
                                        caught = true;
                                    }
                                }
                            }
                        }
                        if fault == 24 {
                            assert!(caught, "n {n} G {groups}: mutant 24 not caught");
                        }
                    }
                }
            }
        }
    }

    fn rots_plus_one(tab: &[Vec<u64>]) -> usize {
        tab[0].len() + 1
    }

    thread_local! {
        /// Lever `Y` in [`chain_with`].
        static SANDWICH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Lever `Y`: the triple with its shared product put into both bits by a CNOT sandwich is
    /// exact for every leaf, costs exactly the Toffolis of the scratch-qubit form, allocates no
    /// scratch qubit, and its mutant 49 (the sandwich's first CNOT dropped) leaves a wrong angle.
    #[test]
    fn triple_sandwich_is_exact_for_every_leaf() {
        let bits = 6;
        for n in [7usize, 9, 13, 16, 30] {
            for groups in [4usize, 6, 8] {
                if groups > n {
                    continue;
                }
                let tab = table((n * 17 + groups) as u64, n, 4, bits);
                for e in 0..n {
                    let (c, tc) = chain_with(n, groups, true, true, &tab, bits, e, 0);
                    SANDWICH.with(|x| x.set(true));
                    let (d, td) = chain_with(n, groups, true, true, &tab, bits, e, 0);
                    SANDWICH.with(|x| x.set(false));
                    assert_eq!(c, tab[e], "zZ n {n} G {groups} leaf {e}");
                    assert_eq!(d, tab[e], "zZY n {n} G {groups} leaf {e}");
                    assert_eq!(
                        tc, td,
                        "n {n} G {groups}: the sandwich changes the Toffolis"
                    );
                }
            }
        }
        // No scratch: the sandwich form allocates nothing beyond the one-hot and the register
        // in a transition whose triple is shared.
        let (n, groups, bits) = (13usize, 4usize, 6usize);
        let tab = table(5, n, 4, bits);
        for sandwich in [false, true] {
            let mut b = Builder::new(1);
            b.declare_uniform(1);
            let e1 = n.div_ceil(groups);
            let hot = Hot {
                q: b.alloc_n(e1),
                g: b.alloc_n(groups - 1),
                e1,
                paired: true,
                fold: None,
                shared: true,
                sandwich,
                rec: None,
            };
            let reg = b.alloc_n(bits);
            let before = b.ops().len();
            let angle = |l: usize, j: usize| tab[l][j];
            transition_by(&mut b, &hot, &reg, n, &angle, None, Some(0));
            let hmr = b.ops()[before..]
                .iter()
                .filter(|o| o.kind == crate::circuit::OperationType::Hmr)
                .count();
            if sandwich {
                assert_eq!(hmr, 0, "the sandwich measures no scratch");
            } else {
                assert!(hmr > 0, "the scratch form is exercised");
            }
        }
        SANDWICH.with(|x| x.set(true));
        let wrong = (0..n).any(|e| {
            std::panic::catch_unwind(|| chain_with(n, groups, true, true, &tab, bits, e, 49))
                .map_or(true, |(seen, _)| seen != tab[e])
        });
        SANDWICH.with(|x| x.set(false));
        assert!(wrong, "sandwich fault 49 not caught");
    }

    /// Lever `Z`: for odd group-bit counts (G = 4, 6, 8) the shared triple is exact for every
    /// leaf and costs at most `ceil(3 m / 2)` Toffolis per transition for a triple with `m`
    /// corrected bits (plus one Toffoli per bit for every other pair), fewer than `z` alone;
    /// mutants 38 (a product's fan-out skipped) and 39 (the shared product left out of the
    /// second bit) leave a wrong angle.
    #[test]
    fn shared_triple_is_exact_for_every_leaf() {
        let bits = 6;
        for n in [7usize, 9, 13, 16, 30] {
            for groups in [4usize, 6, 8] {
                if groups > n {
                    continue;
                }
                let tab = table((n * 17 + groups) as u64, n, 4, bits);
                let mut tz = 0;
                let mut tzz = 0;
                for e in 0..n {
                    let (a, ta) = chain_with(n, groups, true, false, &tab, bits, e, 0);
                    let (c, tc) = chain_with(n, groups, true, true, &tab, bits, e, 0);
                    assert_eq!(a, tab[e], "z n {n} G {groups} leaf {e}");
                    assert_eq!(c, tab[e], "zZ n {n} G {groups} leaf {e}");
                    tz = ta;
                    tzz = tc;
                }
                assert!(tzz <= tz, "n {n} G {groups}: shared {tzz} vs paired {tz}");
                if n.div_ceil(groups) >= 4 {
                    assert!(tzz < tz, "n {n} G {groups}: shared {tzz} vs paired {tz}");
                }
                println!("shared triple n {n} G {groups}: z {tz}, zZ {tzz} Toffolis");
                let trans = (tab[0].len() + 1) as u64;
                let pairs = ((groups - 1) / 2 - 1) as u64;
                let bound = trans * (pairs * bits as u64 + (3 * bits as u64).div_ceil(2));
                assert!(tzz <= bound, "n {n} G {groups}: {tzz} > {bound}");
            }
        }
        for fault in [38u8, 39] {
            let (n, groups) = (13usize, 4usize);
            let tab = table(77, n, 4, bits);
            let wrong = (0..n).any(|e| {
                std::panic::catch_unwind(|| chain_with(n, groups, true, true, &tab, bits, e, fault))
                    .map_or(true, |(seen, _)| seen != tab[e])
            });
            assert!(wrong, "shared fault {fault} not caught");
        }
    }

    /// [`chain`] with lever `Z` switchable.
    #[allow(clippy::too_many_arguments)]
    fn chain_with(
        n: usize,
        groups: usize,
        paired: bool,
        shared: bool,
        tab: &[Vec<u64>],
        bits: usize,
        e: usize,
        fault: u8,
    ) -> (Vec<u64>, u64) {
        let rots = tab[0].len();
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let e1 = n.div_ceil(groups);
        let hot = Hot {
            q: b.alloc_n(e1),
            g: b.alloc_n(groups - 1),
            e1,
            paired,
            fold: None,
            shared,
            sandwich: SANDWICH.with(std::cell::Cell::get),
            rec: None,
        };
        let reg = b.alloc_n(bits);
        let angle = |l: usize, j: usize| tab[l][j];
        let mut seen = Vec::new();
        let mut sim = Sim::new(&b, 7);
        sim.set(hot.q[e % e1], true);
        if e / e1 > 0 {
            sim.set(hot.g[e / e1 - 1], true);
        }
        let mut cur = None;
        FAULT.with(|c| c.set(fault));
        for j in (0..rots).map(Some).chain([None]) {
            let start = b.ops().len();
            transition_by(&mut b, &hot, &reg, n, &angle, cur, j);
            sim.run(&b.ops()[start..]);
            if j.is_some() {
                seen.push(sim.read(&reg));
            }
            cur = j;
        }
        FAULT.with(|c| c.set(0));
        assert_eq!(sim.read(&reg), 0, "register not unloaded");
        sim.set(hot.q[e % e1], false);
        if e / e1 > 0 {
            sim.set(hot.g[e / e1 - 1], false);
        }
        sim.assert_clean();
        (seen, sim.toffolis)
    }

    #[test]
    fn paired_mutants_leave_a_wrong_angle() {
        let bits = 6;
        for (n, groups) in [(9usize, 3usize), (10, 5), (13, 7)] {
            let tab = table(99 + n as u64, n, 4, bits);
            for fault in [20u8, 21] {
                let wrong = (0..n).any(|e| {
                    std::panic::catch_unwind(|| chain(n, groups, true, &tab, bits, e, fault))
                        .map_or(true, |(seen, _)| seen != tab[e])
                });
                assert!(wrong, "n {n} G {groups}: fault {fault} not caught");
            }
        }
    }
}
