//! Low et al. 2025's spectrum-amplified walk step (Phys. Rev. X 15, 041016, Fig. 2 and App. B),
//! as published, on the `sos-sa` specs `reiher-sa-v1` / `li-sa-v1`. Architecture, measured costs
//! and the component-by-component comparison with their Table V: `README.md` here.
//!
//! One step (`FEMOCO_WALK_ARCH=sa-low2025`), lane map `sa-nested-alias-v1`:
//!
//! 1. **Outer PREPARE** (their `PREP_outer`): a clean-QROAM read of the outer alias bucket
//!    (`keep | own item | alt item`), `lt = [draw < keep]`, the alt item swapped in where `lt` is
//!    0. `en_ob = control AND is_ob`, `en_sq = control AND NOT is_ob`, and a `pass` flag.
//! 2. **`nested_inner`**: the block encoding `BE(O)` of the lane's generator
//!    (`inner.rs`: inner PREPARE, RPREP, SELECT, RPREP^dagger, inner PREPARE^dagger), the
//!    `Reflect` on the inner register (their `R_T2`), and the identical copy, which applies
//!    `M^dagger` by reading `pass` (their `Maj^dagger`).
//! 3. **Outer UNPREPARE**, then the harness's walk reflection.
//!
//! What differs from their circuit, and why (README section 3.3): the alias
//! tables need 38 / 42 keep bits under this harness's 0.1 mHa 1-norm rule where they use 9 + 9;
//! the harness cannot swap system qubits, so the spin sector is chosen by running each network
//! on both spins (their controlled spin swap is not expressible); and uniform superpositions are
//! the harness's lanes, not prepared.
pub mod excl;
pub mod inner;
pub mod itemfold;
pub mod itemhot;
pub mod ledger;
pub mod narrow;
pub mod onehot;
pub mod padalias;
pub mod paired;
pub mod pareto;
pub mod qroam;
pub mod range;
pub mod rankdel;
pub mod signnorm;
pub mod tables;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_c1;
#[cfg(test)]
mod tests_combo;
#[cfg(test)]
mod tests_delivery;
#[cfg(test)]
mod tests_floor;
#[cfg(test)]
mod tests_knee;
#[cfg(test)]
mod tests_nonrot;
#[cfg(test)]
mod tests_prep;
#[cfg(test)]
mod tests_rank;
#[cfg(test)]
mod tests_rot;
pub mod toff;

use crate::circuit::{Bit, Builder, Qubit, Reg, SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE};
use crate::lanemap::df_nested::Table;
use crate::lanemap::sa_nested::{self, SaNestedMap};
use crate::lanemap::LaneMap;
use crate::spec::rounding::RoundingClass;
use crate::spec::sa::SaSpec;
use crate::spec::EncodingSpec;
use crate::taxonomy::Family;
use crate::walk::common::arith::{less_than, unless_than};
use crate::walk::common::arith_gated::unless_than_gated;
use crate::walk::common::unary::erase_and;
use crate::walk::shared::lookup::measure;
use inner::{choose_alt, erase_split, InnerRegs};
use ledger::Ledger;
use std::collections::BTreeMap;
use tables::{bits_for, SaTables};

/// Walk parameters that are calibration, not architecture.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Outer and inner alias widths `(k, mu)`.
    pub outer: (u32, u32),
    pub inner: (u32, u32),
    /// log2 of the QROAM block counts of the outer and inner reads (their `k1`, `k2`).
    pub outer_a: usize,
    pub inner_a: usize,
    /// Choose the spin sector with Low et al.'s controlled spin swap (`SpinSwap`, a spec
    /// addition for sos-sa, spec/SPEC-SA.md section 11) instead of running every network on both
    /// spins.
    pub swap: bool,
    /// Angle chunks `C` (`sa-lowq`): the `N - 1` rotation angles of the lane's network are held
    /// `ceil((N - 1) / C)` at a time and streamed through one slot register by XOR transitions
    /// (`inner.rs`), instead of all at once (`C = 1`, as published).
    pub chunks: usize,
    /// Measure the outer alias garbage (keep and the unchosen item, `mu_o + D` qubits) right
    /// after the alt swap, and undo the alias test at the end from a second read of the table
    /// (`sa-lowq`, `FEMOCO_SA_OUTER_ERASE=1`).
    pub outer_erase: bool,
    /// Dense network index `r B + b` (`tables.rs`; `sa-lowq`, on unless `FEMOCO_SA_DENSE=0`):
    /// every angle read is one contiguous unary iteration over `E = R B + N` values, for an
    /// add and a subtract of `b` per copy.
    pub dense: bool,
    /// Toffoli levers on top of the published construction (`sa-toff`; all off for
    /// `sa-low2025`).
    pub tw: Tweaks,
    /// Emit `sa-pareto`'s step (`pareto.rs`) instead of `sa-lowq`'s: the two branches stream the
    /// angles (`chunks`) differently, and each keeps its own implementation. The fields below
    /// are read by `sa-pareto` only.
    pub pareto: bool,
    /// `sa-pareto`'s lean gadgets (README section 5): the one-body flags travel with
    /// the angle word, the inner word drops its `id` flags, and the RPREP iteration skips its
    /// left-only ANDs. Needs `swap`.
    pub lean: bool,
    /// Keep the keep comparisons' carries live so their erasure costs no Toffolis: bit 1 the
    /// outer comparison (held through the whole step), bit 2 the inner one (held through
    /// SELECT). 0 as published (`sa-pareto`).
    pub carries: u32,
    /// Measure the outer alias read's unused slot (`alt` after the swap) as soon as the item is
    /// chosen instead of holding it through SELECT; the outer UNPREPARE then measures the chosen
    /// item too and cancels the phase that depends on the keep comparison with one fixup
    /// controlled by it (`qroam::phase_fixup_ctl`). `D` fewer qubits for about `E(2^k_o)`
    /// more Toffolis (`sa-pareto`).
    pub drop_alt: bool,
    /// The narrowing gadgets for the streamed `sa-pareto` points (`pareto::Narrow`,
    /// `FEMOCO_SA_NARROW`); all off by default.
    pub narrow: pareto::Narrow,
}

/// The `sa-toff` levers (README section 6). Each is a compile-time knob so every one can be
/// measured on its own: `FEMOCO_SA_TWEAKS` is a string of letters, one per lever turned on
/// (default `all` for `sa-toff`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tweaks {
    /// `i`: the inner word stores only `neg`; `id = [b = B]` is an AND of `b`'s literals.
    pub derive_id: bool,
    /// `m`: the alt swaps are undone by measurement: `alt ^= main`, measure `main`, a `CZ(lt,
    /// alt)` per outcome, and the rest into the lookup's fixup (0 Toffolis instead of `w`).
    pub measured_undo: bool,
    /// `c`: compact outer data `q | hi | is_ob | pos_e` (no `lo`); a one-body lane's `b`
    /// register gets `r mod 2^k_i` by `k_i` Toffolis `(is_ob, q_j)` (needs `m`).
    pub compact_outer: bool,
    /// `h`: phase fixups erase their one-hot register by measurement and a small fixup.
    pub hot_erase: bool,
    /// `l`: the inner keep comparator holds its carry ladder until the erasure, which is then
    /// free (`mu_i` more live qubits during SELECT; off by default).
    pub keep_ladder: bool,
    /// `L`: the outer comparator too (`mu_o` more live qubits across the nested block).
    pub outer_ladder: bool,
    /// `x`: item layout of the outer word: the own item's index is the bucket's (copied, not
    /// stored), the alt's is stored, and the inner lookup is indexed by `item 2^k_i + i` over
    /// the squares' range only (needs `c`).
    pub item_outer: bool,
    /// `p`: the write-all-words inner read with shared padding words (`range.rs`; Low et al.
    /// App. B Eq. (B38)): with `2^inner_a = 2^k_i` blocks, the square tables' top padding
    /// buckets are laid out so their words repeat, and the read skips the blocks and swaps that
    /// would only move equal words. Not part of `all`.
    pub pad_share: bool,
    /// `g`: every keep comparison that does not hold its ladder is erased by the outcome-gated
    /// erasure (`common::arith_gated`): `lt` measured, the lower carries recomputed only where
    /// the outcome is 1 (expected `(mu - 1) / 2` Toffolis instead of `mu - 1`, no qubits). Not in
    /// `all`. Alone it applies to the published step's comparisons (`inner.rs`, `emit`).
    pub gated: bool,
    /// `u`: one-hot (unary) RPREP (`onehot.rs`): the network index written one-hot (`R B + N` qubits)
    /// and each angle fanned out by CNOTs into one angle register, instead of the whole angle
    /// word. Not in `all`.
    pub onehot: bool,
    /// `r` (with `u`): the one-hot is written by an in-range iteration (left-only nodes pass
    /// their control down; the identity item reaches a real leaf, harmless around its identity
    /// Majorana), `E - 1` Toffolis per write. Not in `all`.
    pub in_range: bool,
    /// `v` / `w` (with `u`): the one-hot is split into `G = 2` / `3` groups of leaves that share
    /// one `ceil(E / G)`-qubit one-hot plus `G - 1` group bits (`onehot::Hot`): about
    /// `E (1 - 1 / G)` fewer qubits across SELECT, paid by group-controlled corrections of the
    /// angle register, one Toffoli per group bit and register bit whose group difference
    /// changes, per transition. A digit `G` in 2..=9 in the letter string sets any
    /// group count (`v` = `2`, `w` = `3`). 0 (unsplit) otherwise. Not in `all`.
    pub hot_groups: u8,
    /// `d` (with `m`): the outer alias read's unchosen slot is measured right after the swap
    /// (as `D = own ^ alt`, a function of the bucket) instead of at the end, and the part of the
    /// chosen slot's phase that depends on the keep test is cancelled by one fixup controlled by
    /// it (`qroam::phase_fixup_ctl`, about `E(2^k_o) + 2` Toffolis): `kx + h + 2` fewer qubits
    /// through the whole nested block. Not in `all`.
    pub drop_alt: bool,
    /// `k` (with `d`): the outer keep register is measured right after the keep
    /// comparison instead of being held through the nested block, and the comparison is erased
    /// by the outcome-gated re-read of `narrow.rs` (`narrow::gated_lt_erase`, `2^outer_a`
    /// blocks); the re-read's outcomes join the outer read's final fixup. `mu_o` fewer qubits
    /// across SELECT for about half an outer read more Toffolis. Not in `all`.
    pub keep_release: bool,
    /// `j` (with `m`): the inner keep register likewise, in each copy: measured
    /// right after the inner comparison, the comparison erased at PREPARE^dagger by a gated
    /// re-read of `keep` over the lookup's own index range (`narrow::gated_lt_erase_from`,
    /// `2^inner_a` blocks). `mu_i` fewer qubits across SELECT. Not in `all`.
    pub inner_release: bool,
    /// `y` (with `i` and `SpinSwap`): the narrowed Majorana stage. `en_ob`, `en_sq` and `pass`
    /// are not held
    /// across the step: each enable is made and erased around its half of the Majorana, `pass`
    /// is a classical bit toggled at the end of each copy, and `id` keeps no AND chain (erased
    /// by an X measurement gating a recompute of its phase). Not in `all`.
    pub majorana_cut: bool,
    /// `E` (with the unsplit one-hot): the angle register lives inside the
    /// one-hot. For rotation `j`, `w_j` one-hot qubits whose leaves' angles are independent
    /// over GF(2) are turned by CNOTs into the angle bits (`onehot::Pivot`), the `Givens`
    /// reads them, and they are turned back. No separate angle register; Cliffords only. Not in
    /// `all`.
    pub embed: bool,
    /// `z` (with a split one-hot of `G >= 3` groups): the group corrections
    /// are paired. Two group bits `a`, `c` are never both 1, so `(a ^ D_c(h)) (c ^ D_a(h))` gives
    /// both groups' corrections of a register bit with one Toffoli (plus the CNOT fan-out of the
    /// product `D_a D_c`, a function of the one-hot alone): `ceil((G - 1) / 2)` Toffolis per
    /// register bit and transition instead of `G - 1`. Not in `all`.
    pub paired_groups: bool,
    /// `f` (with `r` and a split one-hot): the folded layout
    /// (`onehot::Fold`): whole square rows grouped in classes of `G` rows that share slots, so
    /// the write iterates over `lo` once per class instead of once per row. Not in `all`.
    pub fold: bool,
    /// `a` (with the one-hot): the angle register is unloaded by X
    /// measurement with an all-Clifford phase fixup on the one-hot (`onehot::unload_measured`)
    /// instead of a transition to zero; with a split one-hot it is then also unloaded across the
    /// Majorana and reloaded after it. Not in `all`.
    pub measured_unload: bool,
    /// `b` (with `y`, `i`, `r` and the one-hot): index checkpoint: `b`,
    /// `hi` and `is_ob` are XORed to zero from the one-hot during `V^dagger` and `V` and rebuilt
    /// after (`is_ob` also around the Majorana), `id' = [b = B] AND NOT is_ob` held instead.
    /// Not in `all`.
    pub checkpoint: bool,
    /// `S` (with `i`, `m` and `y`): the inner signs leave at PREPARE. The
    /// square sign is a lane phase, so `(-1)^(control AND NOT is_ob AND s_chosen)` is applied
    /// right after the keep test, as `CZ(t, s_alt) CZ(t AND lt, s_own ^ s_alt)` with
    /// `t = control AND NOT is_ob` (two ANDs, erased by measurement), and both sign slots are
    /// X-measured at once (their contents are functions of the lookup index and join its
    /// fixup). The alt swap then moves `b` alone; the Majorana applies no sign. Not in `all`.
    pub sign_phase: bool,
    /// `R` (with `m`, the plain inner comparison): the inner keep test `lt`
    /// is X-measured right after the swap and recomputed from the held keep at PREPARE^dagger,
    /// where `Z` on the recomputed bit (if the first outcome was 1) cancels the first
    /// measurement's phase. One comparison per copy for one qubit through SELECT. Not in `all`.
    pub lt_release: bool,
    /// `N` (with the SpinSwap SELECT): the spin-select qubit is erased by
    /// measurement right after `F^-s` and made again right before `F^s`. Not in `all`.
    pub spin_unload: bool,
    /// `O` (with `y` and the one-hot): the outer word's `pos_e` slot is
    /// X-measured right after the outer swap (its outcome is the one the final outer fixup
    /// uses), and each copy's Majorana reads `pos_e` from a qubit loaded from the one-hot
    /// (`onehot::transition_by`) and unloaded by measurement (`onehot::unload_measured`, a
    /// Clifford fixup). Not in `all`.
    pub pos_from_hot: bool,
    /// `B` (with `b`): the checkpoint's clearing pass is an X measurement
    /// with a Clifford fixup on the one-hot (`onehot::unload_measured`) instead of a fan-out
    /// transition to zero; the rebuild is unchanged. Not in `all`.
    pub measured_ck: bool,
    /// `W` (with `x`, `d` and `k`): the outer keep test `lt` is X-measured
    /// right after the outer swap. At UNPREPARE a witness `[item = x_o]` (an AND of the item
    /// index's literals against the bucket, `k_o - 1` Toffolis) stands in for it: it equals `lt`
    /// except on self-aliased buckets (`alt = own`, keep 0), where every `lt`-controlled term is
    /// 0. `Z` on the witness cancels the early outcome, `k`'s gated re-read erases it, and the
    /// self-bucket remainder of both outcomes joins the outer fixup. Not in `all`.
    pub outer_witness: bool,
    /// `Q` (with `f`): each folded class flag is
    /// X-measured right after its class's iteration (`onehot::write_folded_early`). Not in `all`.
    pub early_flags: bool,
    /// `H` (with `x` and `m`): the inner alias read and its erasure from a
    /// paired item one-hot (`itemhot.rs`): `n_i (w + 1)` Toffolis per read, no QROAM transient
    /// beyond the item one-hot (`ceil(R C / 2) + 1` qubits), which is present at each copy's start
    /// and end and absent during SELECT. Not in `all`.
    pub item_hot: bool,
    /// `K` (with `H`): the outer item index, `hi` and `is_ob` are cleared
    /// from the item one-hot during the inner read and rebuilt after it. Not in `all`.
    pub item_clear: bool,
    /// `V` (with `H`): the aligned item one-hot (`itemhot::ItemHot::alloc_aligned`):
    /// item `v` at slot `v >> 1`, group `v & 1`, so the group bit is the item register's low bit
    /// and each write iterates over about `R C / 2` slots instead of `R C` items. Not in `all`.
    pub item_align: bool,
    /// `I` (with `V`): the aligned item one-hot is written by an in-place
    /// expansion of its slot index rooted at `NOT is_ob` (one AND per split) and erased by the
    /// measured collapse (each AND erased by measurement, its phase a `CZ` of two qubits still
    /// present): no phase fixup over the item index. Not in `all`.
    pub item_inplace: bool,
    /// `Z` (with `z` and a split one-hot of an even group count `G >= 4`): the
    /// last group pair and the odd group bit share their corrections over pairs of register bits
    /// (`onehot::triple`): three Toffolis per two bits instead of four, so `(G - 1) / 2` per bit
    /// (`G = 4`: 1.5 instead of 2). Not in `all`.
    pub shared_triple: bool,
    /// `X`: the outer alias QROAM read selects its block with the exclusive
    /// multiplexer (`qroam::load_excl`, `excl::mux`) instead of the swap network: `(lambda - 1) w
    /// / 2` Toffolis instead of `(lambda - 1) w`, the low one-hot erased by measurement. Not in
    /// `all`.
    pub excl_select: bool,
    /// `C` (with `r` and a split one-hot): the RPREP one-hot in aligned classes
    /// written in place (`onehot::write_classes`: row one-hot, Clifford fold into class flags and
    /// group bits, in-place `lo` expansions) and erased exactly backwards by measurement
    /// (`onehot::erase_classes`): no fixup over the network index. Replaces `f`. Not in `all`.
    pub class_inplace: bool,
    /// `A` (with `C`): lever `C`'s one-body rows each take their own split depth
    /// (`onehot::ob_depths`), chosen jointly for the fewest slots: on Li G = 4 the first one-body
    /// row stays whole and fills the empty group of the square rows' partial class. Not in `all`.
    pub class_mixed: bool,
    /// `J` (with `b`): the checkpoint's `id'` is made before the RPREP write and
    /// erased after RPREP^dagger, so its AND chains are never live with the one-hot. Not in `all`.
    pub early_id: bool,
    /// `D` (with `H` and `K`): during the inner read the one-body lanes' low item
    /// bits and `pos_e` are stashed in the read's output register, which is 0 on those lanes. Not
    /// in `all`.
    pub stash: bool,
    /// `P` (with the one-hot): a rotation whose angles
    /// use the `pi` bit (`theta = 2 pi a / 2^beta`, bit `beta - 1`) and whose modes no later
    /// rotation touches gets its angle without that bit: `G(theta + pi) = G(theta) Z_p Z_q`, and
    /// `Z_p Z_q` commutes with its own Givens and with every later one, so it is applied at the
    /// chain's edge (before `V^dagger`, after `V`), where the angle register is empty: the bit is
    /// loaded into a scratch qubit from the one-hot, `CZ`s to both modes of both spins, erased by
    /// measurement. The angle register is one qubit narrower when only such rotations needed the
    /// top bit. Not in `all`.
    pub pi_edge: bool,
    /// `Y` (with `Z`): the shared triple's common product goes into its two register
    /// bits by a CNOT sandwich instead of a scratch qubit (`onehot::Hot::sandwich`). Not in `all`.
    pub triple_sandwich: bool,
    /// `s`: global sign normalisation of the Householder vectors. A leaf whose
    /// last chain angle has its top bit set is delivered as the network of `-u` (every
    /// `a_j -> 2^(beta - 1) - a_j` for `j < N - 2`, `a_(N-2) -> a_(N-2) + 2^(beta - 1)`), which
    /// applies the same `n(u, s)` (squares) and, in both copies alike, the same product of
    /// one-body Majoranas. On the est specs every angle then fits `beta - 1` bits: the shared
    /// angle register loses its top qubit (`signnorm.rs`). Not in `all`.
    pub sign_norm: bool,
    /// `U` (with `a` and a split one-hot): the angle register keeps rotation 0's
    /// angle across the Majorana (no measured unload and reload there: `V`'s first load would
    /// pay the group corrections again); the unload after `V` stays measured. Not in `all`.
    pub hold_maj: bool,
    /// `T` (with `V` and `I`): the aligned item one-hot in four groups on the item's
    /// two low bits (`itemhot::ItemHot::alloc_aligned_groups`): about half the slots for two
    /// Toffolis per read bit instead of one. Not in `all`.
    pub item_groups4: bool,
    /// `G`: the inner alias
    /// word is read by the paired unary lookup (`paired.rs`) instead of clean QROAM or the item
    /// one-hot (`H`): the low `inner_a` index bits (inner bucket, then low item bits) written
    /// one-hot, a unary iteration over the rest whose sibling flags are paired (one Toffoli per
    /// pair of groups and word bit), and the one-hot measured. With `G` the knob `inner_a` is the
    /// slot width `s` (`2^s` slots), not a block count. Not in `all`.
    pub paired_lookup: bool,
    /// `M` (with `G`): the paired lookup's low index qubits carry `s` of its `2^s` slots
    /// while the pairs run (each index bit XORed to zero from the one-hot, then swapped with a
    /// slot; rebuilt before the one-hot is measured): `s` fewer live qubits, 0 Toffolis. Not in
    /// `all`.
    pub slot_host: bool,
    /// `e` (with `G`): the chosen item's `hi` and `is_ob` are X-measured right before
    /// the inner read and reloaded from the item index right after it (a small paired lookup,
    /// `toff::reload_fields`; `Z` on each reloaded bit whose outcome was 1 cancels the
    /// measurement's phase): `h + 1` fewer qubits across the read for about one 9-bit lookup per
    /// copy. Not in `all`.
    pub field_release: bool,
    /// `n` (with `G`): the paired lookup's slot one-hot is written by an in-place split
    /// expansion and erased by the measured collapse (Cliffords only) instead of X measurement
    /// plus a phase fixup over the low index bits. Not in `all`.
    pub slot_collapse: bool,
    /// `F` (with `G`): the inner read's measured erasure is the paired phase lookup
    /// (`paired::phase_paired`: split expansion of the low index bits, a range iteration over the
    /// rest with conditioned `CZ`s at each leaf, measured collapse) instead of the QROAM fixup.
    /// Not in `all`.
    pub paired_erase: bool,
    /// `t` (with `H`, `i`, `m` and `y`, in place of `S`): the inner signs as lane
    /// phases without reading both sign flags. The inner read's iteration is rooted at the walk
    /// control (`itemhot::read_into_ext`), so everything it reads is 0 on control-0 lanes (as on
    /// one-body lanes, whose item one-hot is empty) and every phase it applies is controlled for
    /// free. In the same pass `(-1)^(s_alt(item, i))` is applied as a `CZ` of the leaf's gated
    /// parities (Cliffords), and the word stores `delta = s_own ^ s_alt` (0 where `keep = 0`) in
    /// place of the two flags: `CZ(lt, delta)` after the keep test gives `(-1)^(lt delta)`, so
    /// the chosen item's sign `s_alt ^ lt delta` is applied, and `delta` is X-measured at once
    /// (its content joins the erasure's fixup, also rooted at the control). One word bit fewer
    /// per inner index. Not in `all`.
    pub sign_pass: bool,
    /// `o` (with `C`): the RPREP one-hot's classes packed by length. Square rows are
    /// split by their top lo bits like the one-body rows (`onehot::vrows_by`), and the virtual rows
    /// of each split depth are grouped `G` at a time, longest first (`onehot::class_plan`), at the
    /// depths with the fewest slots (`onehot::best_pack`). On Li G = 4 / 6 / 7 the one-hot goes
    /// from 247 / 178 / 178 slots toward `ceil(E / G)`. Not in `all`.
    pub class_pack: bool,
    /// `q<b>` (with a split one-hot of 2 or 3 groups, in place of `a`'s and
    /// `U`'s handling of the register): rank-scheduled delivery (`rankdel.rs`). The copy's rotation
    /// sequence is cut into windows whose word spans (modulo the one-hot) have dimension at most
    /// `b`; each window's span is held live (`b`-ish qubits), its words assembled by CNOTs, and a
    /// switch buys only the new directions (one pair product each). 0 = off. Not in `all`.
    pub rank_hold: u16,
    /// `q<b>_1`: lever `q`'s plan by the furthest-future keep rule (`rankdel::plan_belady`, one
    /// held span per rotation) instead of windows (`q<b>`, mode 0).
    pub rank_mode: u8,
    /// `q<b>_1.<c>` (mode 1 only): a second budget at the gap between `V^dagger` and
    /// `V` (the Majorana): before the Majorana the held span is cut to at most `c` dimensions by
    /// the furthest-future rule, every other row measured out (Clifford fixups, 0 Toffolis), and
    /// the plan continues from the cut span (`rankdel::belady`). The Majorana stage then holds at
    /// most `c` delivery qubits instead of the register plus the held span. 0 = off.
    pub rank_park: u16,
    /// `+` (with `H` and `x`): donor-padded inner tables (`padalias.rs`): every
    /// padding bucket aliases to the item's largest inner item, so the padding word is a
    /// function of the item alone; the item one-hot read and its erasure subtract it, skip the
    /// padding indices, and add it back from the one-hot under the control. Not in `all`.
    pub pad_offset: bool,
    /// `.p` (with `G` and `n`): padding runs. Every square inner table's padding buckets
    /// are fed in aligned subcubes, each by one donor item (`padalias::arrange_runs`, the same
    /// counts), so the paired read's slot one-hot is a pruned expansion in which each fed
    /// subcube is one slot (`paired::load_paired_pruned`): fewer slots on the read stage. Letters
    /// after a `.` are extension levers. Not in `all`.
    pub pad_runs: bool,
    /// `.g` (with `C` and a split one-hot): the last class of `C`'s layout, when it is
    /// all sub-rows of a split one-body row, is grafted into the spare cells of the whole-row
    /// classes (`onehot::plan_grafts`): each sub-row joins a host class's flag and group bit,
    /// its lanes' `lo` is XORed onto the host slots while the one-hot is live, and its flag is
    /// erased by measurement with a Clifford fixup after the host class expands (recomputed by
    /// one Toffoli at the erasure). The one-hot loses that class's slots. Not in `all`.
    pub graft: bool,
    /// `.h<k>` (with `U`): `k` of rotation 0's register bits (the cheapest to reload)
    /// are unloaded by X measurement (Clifford fixup) before the Majorana and reloaded after it,
    /// so the Majorana stage holds `w - k` angle qubits instead of `w`: a cap on `U`'s held span
    /// at the stage where it sets the peak. 0 = off. Not in `all`.
    pub maj_drop: u8,
    /// `.e` (with `H` and a two-group item one-hot): the item one-hot erasure's phase
    /// pass pairs sibling leaves of the inner iteration (`itemhot::phase_rooted_paired`): one
    /// `CCZ` per pair for the group terms instead of one AND per leaf, the quadratic left-over
    /// accumulated in classical bits and applied once. Not in `all`.
    pub erase_pairs: bool,
    /// `-` (with `H V`): the inner read from the folded, `i_0`-refined item one-hot
    /// (`itemfold.rs`): the virtual items `(item, i_0)` in three exclusive groups over the item
    /// one-hot's slots plus `ceil(e1 / 3)` folded slots, iterated over `i >> 1`; `1.5` Toffolis per
    /// word bit per leaf (lever `Z`'s shared triple) over half the leaves, for `ceil(e1 / 3) + 1`
    /// qubits during the read only. Not in `all`.
    pub item_fold: bool,
    /// `_` (with `m` and `t` or `S`): the unchosen inner index is held in the inner
    /// index register through RPREP and SELECT. `i = lt ? chosen : unchosen`, and the chosen index
    /// is held (`bsel`, the RPREP one-hot), so `i ^= lt (chosen ^ unchosen)` puts the unchosen
    /// index there and frees the `k_i`-qubit alt slot; undone at PREPARE^dagger. `k_i` Toffolis
    /// each way per copy for `k_i` qubits off every stage from the alias swap to the measured
    /// undo. Write it before any `q<b>` token (`q<b>_<m>` owns a `_` right after the budget).
    /// Not in `all`.
    pub index_host: bool,
    /// `~` (with `H`, `m`, `_` and `S` or `t`): the inner keep register is X-measured
    /// right after the alias swap (its phase joins the erasure pass), so it is not held through
    /// RPREP and SELECT. `lt` is erased at PREPARE^dagger by lever `j`'s outcome-gated re-read
    /// applied to the item one-hot (`itemhot::gated_lt_erase_hot`): `lt` X-measured, and only
    /// on outcome 1 `keep(item, i)` read again from the rewritten one-hot, `(-1)^[draw < keep]`
    /// applied and the re-read measured into the erasure pass. Not with `R`, `l`, `p`, `D`, `j`.
    /// Not in `all`.
    pub hot_keep_release: bool,
}

impl Tweaks {
    /// Every lever off: the published construction (`sa-low2025`).
    pub const OFF: Self = Self {
        derive_id: false,
        measured_undo: false,
        compact_outer: false,
        hot_erase: false,
        keep_ladder: false,
        outer_ladder: false,
        item_outer: false,
        pad_share: false,
        gated: false,
        onehot: false,
        in_range: false,
        hot_groups: 0,
        drop_alt: false,
        keep_release: false,
        inner_release: false,
        majorana_cut: false,
        embed: false,
        paired_groups: false,
        fold: false,
        measured_unload: false,
        checkpoint: false,
        sign_phase: false,
        lt_release: false,
        spin_unload: false,
        pos_from_hot: false,
        measured_ck: false,
        outer_witness: false,
        early_flags: false,
        item_hot: false,
        item_clear: false,
        item_align: false,
        item_inplace: false,
        shared_triple: false,
        excl_select: false,
        class_inplace: false,
        class_mixed: false,
        early_id: false,
        stash: false,
        pi_edge: false,
        triple_sandwich: false,
        sign_norm: false,
        hold_maj: false,
        item_groups4: false,
        paired_lookup: false,
        slot_host: false,
        field_release: false,
        slot_collapse: false,
        paired_erase: false,
        sign_pass: false,
        class_pack: false,
        rank_hold: 0,
        rank_mode: 0,
        rank_park: 0,
        pad_offset: false,
        pad_runs: false,
        graft: false,
        maj_drop: 0,
        erase_pairs: false,
        item_fold: false,
        index_host: false,
        hot_keep_release: false,
    };

    /// Parses a letter string (`"imch"`, `"all"`, `"none"`).
    #[must_use]
    pub fn parse(s0: &str) -> Self {
        // The letters after the first `.` that is followed by a letter are extension
        // levers, read on their own. A `.` followed by a digit belongs to lever
        // `q<b>_1.<c>` (the Majorana budget), so both syntaxes compose, e.g.
        // `q16_1.13.e`.
        let cut = s0
            .char_indices()
            .find(|&(i, c)| {
                c == '.'
                    && s0[i + 1..]
                        .chars()
                        .next()
                        .is_some_and(|d| !d.is_ascii_digit())
            })
            .map(|(i, _)| i);
        let (s0, ext) = cut.map_or((s0, ""), |i| (&s0[..i], &s0[i + 1..]));
        // Lever `q<b>`: the digits after `q` are its budget, not a group
        // count; they are taken out before the other letters are read.
        let (s, rank_hold, rank_mode, rank_park) = match s0.find('q') {
            Some(i) => {
                let digits: String = s0[i + 1..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                let mut end = i + 1 + digits.len();
                let mut mode = 0u8;
                if s0[end..].starts_with('_') {
                    let md: String = s0[end + 1..]
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect();
                    end += 1 + md.len();
                    mode = md.parse().unwrap_or(0);
                }
                // `.<c>`, the budget at the Majorana gap.
                let mut park = 0u16;
                if s0[end..].starts_with('.') {
                    let pd: String = s0[end + 1..]
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect();
                    end += 1 + pd.len();
                    park = pd.parse().unwrap_or(0);
                }
                let rest = format!("{}{}", &s0[..i], &s0[end..]);
                (rest, digits.parse::<u16>().unwrap_or(0), mode, park)
            }
            None => (s0.to_string(), 0, 0, 0),
        };
        let s = s.as_str();
        let all = s == "all";
        let has = |c: char| all || s.contains(c);
        // A digit `G` in 2..=9: a split one-hot with `G` groups, the general form
        // of `v` (2) and `w` (3); it implies the one-hot. No other lever is a digit.
        let groups = s
            .chars()
            .find_map(|c| c.to_digit(10))
            .filter(|&g| g >= 2)
            .map(|g| g as u8);
        Self {
            derive_id: has('i'),
            measured_undo: has('m') || has('c') || has('x') || (!all && has('d')),
            compact_outer: has('c') || has('x'),
            item_outer: has('x'),
            hot_erase: has('h'),
            keep_ladder: !all && (has('l') || has('L')),
            outer_ladder: !all && has('L'),
            pad_share: !all && has('p'),
            gated: !all && has('g'),
            onehot: !all && (has('u') || has('r') || has('v') || has('w') || groups.is_some()),
            in_range: !all && has('r'),
            hot_groups: if all {
                0
            } else if let Some(g) = groups {
                g
            } else if has('w') {
                3
            } else if has('v') {
                2
            } else {
                0
            },
            drop_alt: !all && has('d'),
            keep_release: !all && has('k'),
            inner_release: !all && has('j'),
            majorana_cut: !all && has('y'),
            embed: !all && has('E'),
            paired_groups: !all && has('z'),
            fold: !all && has('f'),
            measured_unload: !all && has('a'),
            checkpoint: !all && has('b'),
            sign_phase: !all && has('S'),
            lt_release: !all && has('R'),
            spin_unload: !all && has('N'),
            pos_from_hot: !all && has('O'),
            measured_ck: !all && has('B'),
            outer_witness: !all && has('W'),
            early_flags: !all && has('Q'),
            item_hot: !all && has('H'),
            item_clear: !all && has('K'),
            item_align: !all && has('V'),
            item_inplace: !all && has('I'),
            shared_triple: !all && has('Z'),
            excl_select: !all && has('X'),
            class_inplace: !all && has('C'),
            class_mixed: !all && has('A'),
            early_id: !all && has('J'),
            stash: !all && has('D'),
            pi_edge: !all && has('P'),
            triple_sandwich: !all && has('Y'),
            sign_norm: !all && has('s'),
            hold_maj: !all && has('U'),
            item_groups4: !all && has('T'),
            paired_lookup: !all && (has('G') || has('M')),
            slot_host: !all && has('M'),
            // (`e` and `n` only with the paired lookup: "none" contains both letters.)
            field_release: !all && has('e') && (has('G') || has('M')),
            slot_collapse: !all && has('n') && (has('G') || has('M')),
            paired_erase: !all && has('F'),
            sign_pass: !all && has('t'),
            class_pack: !all && has('o') && has('C'),
            rank_hold: if all { 0 } else { rank_hold },
            rank_mode: if all { 0 } else { rank_mode },
            rank_park: if all { 0 } else { rank_park },
            pad_offset: !all && has('+'),
            pad_runs: !all && ext.contains('p'),
            graft: !all && ext.contains('g'),
            erase_pairs: !all && ext.contains('e'),
            maj_drop: if all {
                0
            } else {
                ext.find('h').map_or(0, |i| {
                    ext[i + 1..]
                        .chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0)
                })
            },
            item_fold: !all && has('-'),
            index_host: !all && has('_'),
            hot_keep_release: !all && has('~'),
        }
    }

    /// The `sa-toff` default: `FEMOCO_SA_TWEAKS` at build time, else every lever but `l`.
    #[must_use]
    pub fn toff() -> Self {
        Self::parse(option_env!("FEMOCO_SA_TWEAKS").unwrap_or("all"))
    }

    /// Whether any lever is on.
    #[must_use]
    pub fn any(&self) -> bool {
        *self != Self::default()
    }

    /// Whether any lever other than `g` is on (`g` alone keeps the published step's emitter).
    #[must_use]
    pub fn any_toff(&self) -> bool {
        Self {
            gated: false,
            ..*self
        }
        .any()
    }
}

fn env_u32(v: Option<&str>) -> Option<u32> {
    v.and_then(|s| s.parse().ok())
}

impl Params {
    /// The keep split that passes the spec's rounding rule most cheaply with largest-remainder
    /// counts: for the 0.1 mHa 1-norm rule (`tests::keep_bit_scan`: an inner keep bit costs
    /// about 35 Toffolis per step, an outer one about 6, so the outer width is raised until the
    /// inner one drops); for the `sos-ground-cs-v1` specs (spec/SPEC-SA.md section 12) the
    /// cheapest `sa-toff imchxL` split whose ground-energy bound meets the spec's budget
    /// (`tests::rigbits_scan`); and Low et al.'s
    /// QROAM block counts (`k1 = 2`, `k2 = 4` Reiher / `5` Li). Build-time overrides:
    /// `FEMOCO_SA_MU_O`, `FEMOCO_SA_MU_I`, `FEMOCO_SA_OUTER_A`, `FEMOCO_SA_INNER_A`.
    #[must_use]
    pub fn for_spec(spec: &SaSpec) -> Self {
        let k_o = bits_for(spec.outer_items() as u64) as u32;
        let k_i = bits_for(spec.b as u64 + 1) as u32;
        let ground = matches!(spec.rounding, crate::spec::sa::Rounding::Ground { .. });
        let estimated = spec.rounding_class != RoundingClass::Rigorous;
        let (mu_o, mu_i) = match (ground, spec.n, k_o, k_i, spec.beta) {
            // Under the estimated class (`*-sa-est-v1`, spec/SPEC-SA.md section 14): Low et al.'s 9 + 9.
            _ if estimated => (9, 9),
            (false, 54, 9, 5, _) => (19, 19),
            (false, 76, 9, 6, _) => (20, 21),
            // reiher-sa-gb-v1 (0.25 mHa): bound 2.499e-4; li-sa-gb-v1 (0.25 mHa): 2.158e-4;
            // reiher-sa-gb-r15-v1 (0.125 mHa): 1.2495e-4; li-sa-gb-r14-v1 (0.14 mHa): 1.070e-4.
            (true, 54, 9, 5, 16) => (18, 15),
            (true, 76, 9, 6, 15) => (17, 17),
            (true, 54, 9, 5, 15) => (16, 17),
            (true, 76, 9, 6, 14) => (18, 18),
            _ => {
                let mu = (61 - k_o - k_i) / 2;
                (mu.min(31), (61 - k_o - k_i - mu).min(31))
            }
        };
        let mu_o = env_u32(option_env!("FEMOCO_SA_MU_O")).unwrap_or(mu_o);
        let mu_i = env_u32(option_env!("FEMOCO_SA_MU_I")).unwrap_or(mu_i);
        let outer_a = env_u32(option_env!("FEMOCO_SA_OUTER_A")).map_or(2, |v| v as usize);
        let inner_a = env_u32(option_env!("FEMOCO_SA_INNER_A"))
            .map_or(k_i.saturating_sub(1) as usize, |v| v as usize);
        Self {
            outer: (k_o, mu_o),
            inner: (k_i, mu_i),
            outer_a,
            inner_a,
            swap: true,
            chunks: 1,
            outer_erase: false,
            dense: false,
            tw: Tweaks::default(),
            pareto: false,
            lean: false,
            carries: 0,
            drop_alt: false,
            narrow: pareto::Narrow::OFF,
        }
    }

    /// `sa-pareto`: [`Params::for_spec`] with the lean gadgets, and the build-time knobs
    /// `FEMOCO_SA_CHUNKS` (angle chunks `C`, default 1), `FEMOCO_SA_CARRIES` (keep the
    /// comparison carries: 1 outer, 2 inner, 3 both; default 0), `FEMOCO_SA_DROP_ALT`
    /// ([`Params::drop_alt`], default 0) and `FEMOCO_SA_LEAN` (default 1), besides
    /// [`Params::for_spec`]'s.
    #[must_use]
    pub fn pareto(spec: &SaSpec) -> Self {
        Self {
            pareto: true,
            lean: env_u32(option_env!("FEMOCO_SA_LEAN")).is_none_or(|v| v != 0),
            carries: env_u32(option_env!("FEMOCO_SA_CARRIES")).unwrap_or(0),
            chunks: env_u32(option_env!("FEMOCO_SA_CHUNKS")).map_or(1, |v| v.max(1) as usize),
            drop_alt: env_u32(option_env!("FEMOCO_SA_DROP_ALT")).is_some_and(|v| v != 0),
            narrow: pareto::Narrow::parse(option_env!("FEMOCO_SA_NARROW").unwrap_or("")),
            ..Self::for_spec(spec)
        }
    }
}

/// The lane map this walk declares: `sa_nested::build`'s largest-remainder tables, with every
/// one-body inner table replaced by the equivalent `keep = 0, alt = i mod 2` table (the same
/// counts, exactly half the lanes on each item), so a one-body lane's inner item is the low
/// inner index bit and needs no lookup.
///
/// # Errors
/// As `sa_nested::build`.
pub fn lane_map(spec: &SaSpec, p: Params) -> Result<SaNestedMap, String> {
    let m = sa_nested::build(spec, p.outer, p.inner)?;
    let (k_i, mu_i) = p.inner;
    let half = Table {
        k: k_i,
        mu: mu_i,
        keep: vec![0; 1 << k_i],
        alt: (0..1u32 << k_i).map(|i| i & 1).collect(),
    };
    let mut inner = m.inner;
    for t in inner.iter_mut().take(spec.n) {
        *t = half.clone();
    }
    // Lever `+`: the square tables' padding aliased to each item's largest inner
    // item (the same counts).
    if p.tw.pad_offset {
        for t in inner.iter_mut().skip(spec.n) {
            *t = padalias::arrange(t, spec.b + 1);
        }
    }
    // Lever `.p`: every padding subcube fed by one donor (the same counts).
    if p.tw.pad_runs {
        assert!(
            !p.tw.pad_offset,
            "lever .p and lever + arrange the padding differently"
        );
        for t in inner.iter_mut().skip(spec.n) {
            *t = padalias::arrange_runs(t, spec.b + 1).0;
        }
    }
    // `sa-toff` lever `p` (range.rs): the square tables' padding laid out for the padded read.
    if let Some(plan) = pad_plan_of(spec, p, &inner) {
        for t in inner.iter_mut().skip(spec.n) {
            *t = range::pad_table(t, spec.b + 1, plan);
        }
    }
    SaNestedMap::new(m.lambda_decl, m.outer, inner, spec)
}

/// The pad plan of lever `p` for these inner tables (`None` when the lever is off, the read has
/// fewer than `2^k_i` blocks, or no plan fits every square table).
#[must_use]
pub fn pad_plan_of(spec: &SaSpec, p: Params, inner: &[Table]) -> Option<range::PadPlan> {
    if !p.tw.pad_share || p.inner_a < p.inner.0 as usize {
        return None;
    }
    range::pad_plan(&inner[spec.n..], spec.b + 1)
}

/// Emits the walk step for `spec` (an sos-sa spec): `swap` picks the spin sector with
/// `SpinSwap` (architecture `sa-low2025`, as published); without it every network runs on both
/// spins (`sa-low2025-bothspin`, which needs no spec addition).
///
/// # Panics
/// If `spec` is not an [`SaSpec`] or the lane map cannot be built.
pub fn build(spec: &dyn EncodingSpec, b: &mut Builder, swap: bool) -> Box<dyn LaneMap> {
    build_toff(spec, b, swap, Tweaks::default())
}

/// [`build`] with the `sa-toff` levers `tw` (architecture `sa-toff`).
///
/// # Panics
/// As [`build`].
pub fn build_toff(
    spec: &dyn EncodingSpec,
    b: &mut Builder,
    swap: bool,
    tw: Tweaks,
) -> Box<dyn LaneMap> {
    let sa = spec
        .as_any()
        .downcast_ref::<SaSpec>()
        .expect("the sa-low2025 walk needs an sos-sa spec");
    let p = Params {
        swap,
        tw,
        ..Params::for_spec(sa)
    };
    build_with(sa, b, p)
}

/// Emits the step with explicit parameters (`sa-lowq` and the tests).
///
/// # Panics
/// If the lane map cannot be built.
pub fn build_with(sa: &SaSpec, b: &mut Builder, p: Params) -> Box<dyn LaneMap> {
    let map = lane_map(sa, p).expect("sa lane map");
    b.declare_uniform(map.uniform_bits());
    let _ = emit(sa, &map, b, p);
    Box::new(map)
}

/// Emits the step for a given lane map (`map.uniform_bits()` must already be declared) and
/// returns the per-stage ledger.
pub fn emit(spec: &SaSpec, map: &SaNestedMap, b: &mut Builder, p: Params) -> Ledger {
    if p.tw.any_toff() {
        assert!(
            !p.pareto,
            "the sa-toff levers and sa-pareto are separate walks"
        );
        return toff::emit(spec, map, b, p);
    }
    if p.pareto {
        return pareto::emit(spec, map, b, p);
    }
    let t = SaTables::with_index(spec, map, p.dense);
    let mut led = Ledger::default();
    let (k_o, mu_o) = (map.outer.k, map.outer.mu);
    let (u_o, w) = (map.outer_bits(), map.inner_width());
    let k_i = map.inner[0].k;
    let mu_i = map.inner[0].mu;
    let range = |b: &Builder, lo: u32, hi: u32| (lo..hi).map(|i| b.uniform(i)).collect::<Vec<_>>();
    let (index_o, draw_o) = (range(b, 0, k_o), range(b, k_o, k_o + mu_o));
    let s1 = b.uniform(k_o + mu_o);
    let (index_i, draw_i) = (
        range(b, u_o, u_o + k_i),
        range(b, u_o + k_i, u_o + k_i + mu_i),
    );
    let s0 = b.uniform(u_o + w - 1);
    // Registers first: a copy may not declare them. The angle qubits are live only from their
    // first load (src/sim/liveness.rs).
    let slots = t.slots(p.chunks);
    let angle_q = b.alloc_n(slots.bits());
    let angle_regs: Vec<Reg> = (0..slots.width.len())
        .map(|j| b.register(&angle_q[slots.at[j]..slots.at[j] + slots.width[j]]))
        .collect();
    let inner_reg = b.inner_register(u_o, w);

    led.start(b);
    b.segment(SEG_PREPARE);
    let d = t.outer_data_bits();
    let buckets = 1u64 << k_o;
    let words = |i: u64| t.outer_word(i);
    let read = qroam::load(
        b,
        &index_o,
        buckets,
        mu_o as usize + 2 * d,
        &words,
        p.outer_a,
    );
    let all = read.out.clone();
    let keep: Vec<Qubit> = all[..mu_o as usize].to_vec();
    let main: Vec<Qubit> = all[mu_o as usize..mu_o as usize + d].to_vec();
    let alt: Vec<Qubit> = all[mu_o as usize + d..].to_vec();
    led.stage(b, "outer: alias QROAM read");
    let lt = less_than(b, &draw_o, &keep);
    choose_alt(b, lt, &main, &alt);
    let (lo_at, hi_at, q_at, ob_at, pos_at) = t.outer_fields();
    let is_ob = main[ob_at];
    let control = b.control();
    let en_ob = b.alloc();
    b.ccx(control, is_ob, en_ob);
    let en_sq = b.alloc();
    b.cx(control, en_sq);
    b.cx(en_ob, en_sq);
    let pass = b.alloc();
    // Outer garbage out early: keep held keep(i), the alt register the unchosen item. Their
    // phases are cancelled at the end on a second read (`unprepare_from_reread`).
    let early = p
        .outer_erase
        .then(|| (measure(b, keep.clone()), measure(b, alt.clone())));
    led.stage(b, "outer: keep test, alt swap, enables");

    b.segment(SEG_SELECT);
    let regs = InnerRegs {
        index: index_i,
        draw: draw_i,
        s0,
        s1,
        lo: main[lo_at..hi_at].to_vec(),
        hi: main[hi_at..q_at].to_vec(),
        q: main[q_at..ob_at].to_vec(),
        pos_e: main[pos_at],
        is_ob,
        en_ob,
        en_sq,
        pass,
        angle_q: &angle_q,
        angle_regs: &angle_regs,
        slots: &slots,
    };
    b.nested_inner(inner_reg, |b| inner::copy(b, &t, &regs, p, &mut led));
    led.start(b);

    b.segment(SEG_UNPREPARE);
    b.free(pass);
    b.cx(en_ob, en_sq);
    b.cx(control, en_sq);
    b.free(en_sq);
    erase_and(b, control, is_ob, en_ob, false);
    if let Some((mk, ma)) = early {
        let junk = read.into_junk();
        led.stage(b, "outer^-1: enables");
        let read2 = qroam::load(
            b,
            &index_o,
            buckets,
            mu_o as usize + 2 * d,
            &words,
            p.outer_a,
        );
        led.stage(b, "outer^-1: second alias QROAM read");
        let mu = mu_o as usize;
        let out2 = read2.out.clone();
        unprepare_from_reread(
            b,
            lt,
            main,
            (&out2[..mu], &out2[mu..mu + d], &out2[mu + d..]),
            (&mk, &ma),
        );
        erase_lt(b, &draw_o, &out2[..mu], lt, p.tw.gated);
        led.stage(b, "outer^-1: phases, keep test");
        qroam::erase_with(b, &index_o, read2, junk, &words, erase_split(buckets));
        led.stage(b, "outer^-1: alias reads erased");
    } else {
        choose_alt(b, lt, &main, &alt);
        erase_lt(b, &draw_o, &keep, lt, p.tw.gated);
        led.stage(b, "outer^-1: alt swap, keep test, enables");
        qroam::erase(b, &index_o, read, &words, erase_split(buckets));
        led.stage(b, "outer^-1: alias read erased");
    }
    angle_q.into_iter().for_each(|q| b.free(q));
    led
}

/// Erases a keep comparison `lt = [draw < keep]`: [`unless_than`], or the outcome-gated erasure
/// (`Tweaks::gated`, `common::arith_gated`).
pub fn erase_lt(b: &mut Builder, draw: &[Qubit], keep: &[Qubit], lt: Qubit, gated: bool) {
    if gated {
        unless_than_gated(b, draw, keep, lt);
    } else {
        unless_than(b, draw, keep, lt);
    }
}

/// Undoes the outer alias test when its garbage was measured after the swap: `keep` (outcomes
/// `mk`) held `keep(i)`, the alt register (outcomes `ma`) the unchosen item, and `main` holds the
/// chosen item, `own(i)` where `lt` is 1 and `alt(i)` where it is 0. `(keep2, own2, alt2)` is a
/// second read of bucket `i`. `main` is measured too (outcomes `mm`); the three measurements left
/// the phase `mk.keep ^ ma.other ^ mm.chosen = mk.keep ^ f ^ lt g`, with
/// `f = ma.own ^ mm.alt` and `g = (ma ^ mm).(own ^ alt)`, which conditioned `Z`s on the second
/// read cancel: `Z^mk` on `keep2`, `Z^ma` on `own2`, `Z^mm` on `alt2`, and, with `alt2` holding
/// `own ^ alt` for a moment, `CZ(lt, .)` on each of its bits conditioned on `ma` and on `mm`.
/// No Toffolis; `lt` and the second read remain for the caller.
fn unprepare_from_reread(
    b: &mut Builder,
    lt: Qubit,
    main: Vec<Qubit>,
    (keep2, own2, alt2): (&[Qubit], &[Qubit], &[Qubit]),
    (mk, ma): (&[Bit], &[Bit]),
) {
    let mm = measure(b, main);
    for (&q, &m) in keep2.iter().zip(mk) {
        b.z_if(q, m);
    }
    for ((&o, &a), (&m_a, &m_m)) in own2.iter().zip(alt2).zip(ma.iter().zip(&mm)) {
        b.z_if(o, m_a);
        b.z_if(a, m_m);
        b.cx(o, a);
        b.cz_if(lt, a, m_a);
        b.cz_if(lt, a, m_m);
        b.cx(o, a);
    }
}

/// `sa-lowq`: the published step with the angles streamed in `C` chunks (`FEMOCO_SA_CHUNKS`,
/// default 2) and the controlled spin swap. `FEMOCO_SA_INNER_A` (and the other `Params`
/// overrides) apply as for `sa-low2025`.
///
/// # Panics
/// If `spec` is not an [`SaSpec`] or the lane map cannot be built.
pub fn build_lowq(spec: &dyn EncodingSpec, b: &mut Builder) -> Box<dyn LaneMap> {
    let sa = spec
        .as_any()
        .downcast_ref::<SaSpec>()
        .expect("the sa-lowq walk needs an sos-sa spec");
    build_with(sa, b, Params::for_lowq(sa))
}

impl Params {
    /// `sa-lowq`'s parameters: [`Params::for_spec`] with the angles streamed in `chunks`
    /// (`FEMOCO_SA_CHUNKS`, default 2), the outer garbage measured early (`FEMOCO_SA_OUTER_ERASE`,
    /// default 1), the dense network index (`FEMOCO_SA_DENSE`, default 1), and, once the angles
    /// are streamed, at most `2^4` inner QROAM blocks (their transient then sets the peak;
    /// `FEMOCO_SA_INNER_A` overrides).
    #[must_use]
    pub fn for_lowq(spec: &SaSpec) -> Self {
        let on = |v: Option<&str>| env_u32(v).is_none_or(|v| v != 0);
        let chunks = env_u32(option_env!("FEMOCO_SA_CHUNKS")).map_or(2, |v| v as usize);
        let base = Self::for_spec(spec);
        let inner_a = if option_env!("FEMOCO_SA_INNER_A").is_none() && chunks > 1 {
            base.inner_a.min(4)
        } else {
            base.inner_a
        };
        Self {
            chunks,
            inner_a,
            outer_erase: on(option_env!("FEMOCO_SA_OUTER_ERASE")),
            dense: on(option_env!("FEMOCO_SA_DENSE")),
            ..base
        }
    }
}

/// The declared family of `sa-lowq`: `sa-low2025-spinswap`'s axes (streaming the angles changes
/// no taxonomy axis), its own name, that family as parent.
#[must_use]
pub fn family_lowq() -> Family {
    Family {
        name: "sa-lowq-stream".into(),
        parent: Some("sa-low2025-spinswap".into()),
        ..family(true)
    }
}

/// The declared family (taxonomy 1.3.0): `swap` names the two spin-sector choices, which share
/// every axis value (calibration within one family's axes, different names).
#[must_use]
pub fn family(swap: bool) -> Family {
    let axes: BTreeMap<String, String> = [
        ("encoding", "sos-sa"),
        ("lane_map", "sa-nested-alias-v1"),
        ("lookup", "qroam-clean"),
        ("select", "givens-sa-nested"),
        ("uncompute", "measurement-based"),
        ("reuse", "serial"),
        ("rotation", "phase-gradient-givens"),
    ]
    .into_iter()
    .map(|(a, v)| (a.to_string(), v.to_string()))
    .collect();
    Family {
        taxonomy_version: "1.3.0".into(),
        name: if swap {
            "sa-low2025-spinswap".into()
        } else {
            "sa-low2025-bothspin".into()
        },
        parent: None,
        axes,
    }
}

/// The declared family of `sa-toff`: the published construction's axes (every lever is a
/// layout or erasure choice within them), named apart and parented on `sa-low2025-spinswap`.
#[must_use]
pub fn family_toff() -> Family {
    Family {
        name: "sa-low2025-toff".into(),
        parent: Some("sa-low2025-spinswap".into()),
        ..family(true)
    }
}

/// Emits the `sa-pareto` walk step ([`Params::pareto`]).
///
/// # Panics
/// If `spec` is not an [`SaSpec`] or the lane map cannot be built.
pub fn build_pareto(spec: &dyn EncodingSpec, b: &mut Builder) -> Box<dyn LaneMap> {
    let sa = spec
        .as_any()
        .downcast_ref::<SaSpec>()
        .expect("the sa-pareto walk needs an sos-sa spec");
    build_with(sa, b, Params::pareto(sa))
}

/// `sa-pareto`'s family: the published construction's axes, with the lean gadgets and the
/// chunked angle loads as calibration of it (parent `sa-low2025-spinswap`).
#[must_use]
pub fn family_pareto() -> Family {
    Family {
        name: "sa-pareto".into(),
        parent: Some("sa-low2025-spinswap".into()),
        ..family(true)
    }
}
