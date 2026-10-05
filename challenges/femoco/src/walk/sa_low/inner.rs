//! One inner copy: `BE(O)` of the lane's generator, emitted once and repeated byte for byte after
//! the `Reflect` by `Builder::nested_inner` (spec/SPEC-SA.md section 5.2). Low et al.'s Fig. 2:
//! inner PREP, RPREP, SELECT (rotate, Majorana, rotate back), RPREP^dagger, inner PREP^dagger.
//!
//! On a lane whose outer item is square `q = (r, c)` (or one-body `r`, spin `s1`) and whose inner
//! value is `a = i | draw << k_i | s0 << (k_i + mu_i)`:
//!
//! 1. **Inner PREPARE** (squares). A clean-QROAM read at `x = q 2^k_i + i` (`2^inner_a` blocks
//!    over the low bits of `i`, their `k2`) writes `keep | own flags | alt b | alt flags`. The own
//!    item's `b` is `i` itself, copied. `lt = [draw < keep]` and the alt is swapped in where `lt`
//!    is 0. On a one-body lane the read is past the table (`q` is all ones), so everything reads
//!    0, `lt` is 0, and `b` ends up 0; the one-body inner item is `x = i mod 2`, the lane map's
//!    choice (`super::lane_map`).
//! 2. **RPREP.** `lo ^= outer lo` makes `idx = lo | hi << k_i` the network index
//!    (`tables.rs`), and one unary-iteration read loads all `N - 1` angles of network `idx`.
//! 3. **SELECT.** `V^dagger` as `Z_S V_rev Z_S` (the same angle registers, `shared::angles`),
//!    the controlled Majorana, then `V`. Each Givens is applied on both spins from the same
//!    angle register: the other spin's rotations commute with the Majorana and cancel, so the
//!    spin bit only steers the Majorana (the harness cannot swap system qubits, so Low et al.'s
//!    controlled spin swap is replaced by the second spin's network).
//!    - square: `M_(b, s0) = -sign(w_b) Z(u_b, s0)`: `CZ(en_sq, neg)` and a `Z` on system qubit
//!      `s0` where `en_sq AND NOT id`; the identity item applies only its sign.
//!    - one-body: `M_x = gamma(u_r, s1, 0)` or `+-i gamma(u_r, s1, 1)`; the second copy applies
//!      `M^dagger`, which differs by `-1` on `x = 1`: `CZ(sel_x, pass)`.
//! 4. **RPREP^dagger**: the angles are measured out (the registers stay allocated in the builder,
//!    `hmr_keep`) and their phase fixed; `lo ^= outer lo`.
//! 5. **Inner PREPARE^dagger**: swap back, erase the comparison, clear the copy of `i`, erase the
//!    read by measurement. Toggle `pass`.
use super::ledger::Ledger;
use super::qroam;
use super::tables::{SaTables, Slots};
use super::Params;
use crate::circuit::{Builder, Qubit, Reg};
use crate::walk::common::arith::{cswap_reg, less_than};
use crate::walk::common::lookup::erase_lookup_keep;
use crate::walk::common::unary::erase_and;
use crate::walk::common::unary::iterate;
use crate::walk::shared::arith::{add_into, sub_from};
use crate::walk::shared::lookup::{set_bits, xor, xor_lookup, Word};

/// What a copy reads, all prepared outside it.
pub struct InnerRegs<'a> {
    /// Inner uniform bits: alias index `i` (`k_i`), keep draw (`mu_i`), and the spin bit `s0`.
    pub index: Vec<Qubit>,
    pub draw: Vec<Qubit>,
    pub s0: Qubit,
    /// The outer spin bit (one-body generators only).
    pub s1: Qubit,
    /// Outer item fields (`tables.rs`).
    pub lo: Vec<Qubit>,
    pub hi: Vec<Qubit>,
    pub q: Vec<Qubit>,
    pub pos_e: Qubit,
    /// The outer item is a one-body generator (not gated by the control).
    pub is_ob: Qubit,
    /// `control AND is_ob` and `control AND NOT is_ob`.
    pub en_ob: Qubit,
    pub en_sq: Qubit,
    /// 0 in the first copy, 1 in the second (each copy toggles it at its end).
    pub pass: Qubit,
    /// The angle slot qubits and one register per slot (`slots.g` of them).
    pub angle_q: &'a [Qubit],
    pub angle_regs: &'a [Reg],
    pub slots: &'a Slots,
}

/// Low index bits split off when a lookup of `limit` values is erased by measurement: the `h`
/// that minimizes the fix-up's `2 * 2^h + limit / 2^h` Toffolis.
#[must_use]
pub fn erase_split(limit: u64) -> usize {
    (0..20)
        .min_by_key(|&h| 2 * (1u64 << h) + limit.div_ceil(1 << h))
        .unwrap_or(0)
}

/// XORs network `idx = lo | hi << k_i`'s angle word into `out` by a ragged unary iteration: over
/// `hi` (`R` square rows, then the one-body rows), and inside each row over `lo` up to that row's
/// length (`B` for a square row, so the identity item and the padding are never visited). Low et
/// al.'s `N + R B` RPREP iteration, instead of `R 2^k_i + N` over the padded index.
///
/// `data(v)` is the word XORed in at index value `v` (a chunk's angles, or the XOR of two
/// chunks' for a transition).
pub fn ragged_read(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    out: &[Qubit],
    data: &dyn Fn(u64) -> Word,
) {
    let s = t.spec;
    let row = 1u64 << t.k_i;
    let rows = t.net_limit().div_ceil(row);
    let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
        let len = if (hv as usize) < s.r {
            s.b as u64
        } else {
            (t.net_limit() - hv * row).min(row)
        };
        let mut inner = |b: &mut Builder, v: u64, f: Qubit| {
            for j in set_bits(&data(hv * row + v), out.len()) {
                b.cx(f, out[j]);
            }
        };
        iterate(b, Some(flag), lo, len, &mut inner);
    };
    iterate(b, None, hi, rows, &mut leaf);
}

/// Swaps the alt data into `main` where `lt` is 0. Self-inverse.
pub fn choose_alt(b: &mut Builder, lt: Qubit, main: &[Qubit], alt: &[Qubit]) {
    b.x(lt);
    cswap_reg(b, lt, main, alt);
    b.x(lt);
}

/// Emits one inner copy.
pub fn copy(b: &mut Builder, t: &SaTables<'_>, r: &InnerRegs<'_>, p: Params, led: &mut Ledger) {
    let k_i = t.k_i;
    let mu = r.draw.len();
    led.start(b);
    // 1. Inner PREPARE.
    let index: Vec<Qubit> = [&r.index[..], &r.q[..]].concat();
    let limit = t.inner_limit();
    let words = |x: u64| t.inner_word(x);
    let read = qroam::load(
        b,
        &index,
        limit,
        t.inner_word_bits(),
        &words,
        p.inner_a.min(k_i),
    );
    let out = read.out.clone();
    led.stage(b, "copy: inner alias QROAM read");
    let keep = &out[..mu];
    let own_flags = &out[mu..mu + 2];
    let alt = &out[mu + 2..];
    let bsel = b.alloc_n(k_i);
    for (&i, &q) in r.index.iter().zip(&bsel) {
        b.cx(i, q);
    }
    let main: Vec<Qubit> = [&bsel[..], own_flags].concat();
    let lt = less_than(b, &r.draw, keep);
    choose_alt(b, lt, &main, alt);
    let (neg, id) = (main[k_i], main[k_i + 1]);
    led.stage(b, "copy: inner keep test and alt swap");

    // 2. RPREP: idx = lo | hi (or, dense, the outer base plus b); the first chunk of angles (all
    // of them when C = 1).
    let idx: Vec<Qubit> = if t.dense {
        add_into(b, &bsel, &r.hi);
        r.hi.clone()
    } else {
        for (&o, &q) in r.lo.iter().zip(&bsel) {
            b.cx(o, q);
        }
        [&bsel[..], &r.hi[..]].concat()
    };
    let e = t.net_limit();
    let sl = r.slots;
    let states = sl.states();
    let word_of = |v: u64, si: usize| t.chunk_word(v, states[si], sl);
    let load_angles = |b: &mut Builder, data: &dyn Fn(u64) -> Word| {
        if t.dense {
            xor_lookup(b, &idx, e, r.angle_q, data);
        } else {
            ragged_read(b, t, &bsel, &r.hi, r.angle_q, data);
        }
    };
    load_angles(b, &|v| word_of(v, 0));
    led.stage(b, "copy: RPREP angle read");

    // 3. SELECT, the angles streamed chunk by chunk: V^dagger over chunks C-1 .. 0, the
    // Majorana, V over chunks 0 .. C-1, each change of chunk one XOR transition.
    let spins: &[usize] = if p.swap { &[0] } else { &[0, 1] };
    let odd = t.odd_class();
    let z_s = |b: &mut Builder| {
        for &m in &odd {
            for &s in spins {
                let q = b.system(2 * m + s);
                b.z(q);
            }
        }
    };
    let rotate = |b: &mut Builder, c: usize, reverse: bool| {
        let js = c * sl.g..((c + 1) * sl.g).min(t.modes.len());
        let order: Vec<usize> = if reverse {
            js.rev().collect()
        } else {
            js.collect()
        };
        for j in order {
            let (pm, qm) = t.modes[j];
            for &s in spins {
                b.givens_modes(2 * pm + s, 2 * qm + s, r.angle_regs[j - c * sl.g]);
            }
        }
    };
    let sel = p.swap.then(|| spin_select(b, r));
    if let Some(g) = sel {
        // F^dagger before, F after: F^s V M V^dagger F^-s. (F^dagger = Z_S1 F Z_S1 would hand the
        // tracker two 54-qubit Z frames per copy, past its 512-factor limit.)
        b.spin_swap_dg(g);
        led.stage(b, "copy: SELECT controlled spin swap F^-s");
    }
    let chunks = sl.chunks;
    for (si, &c) in states.iter().enumerate() {
        if si > 0 {
            let diff = |v: u64| xor(&word_of(v, si - 1), &word_of(v, si));
            load_angles(b, &diff);
            led.stage(b, "copy: RPREP angle transition");
        }
        if si < chunks {
            if si == 0 {
                z_s(b);
            }
            rotate(b, c, true);
            if si + 1 == chunks {
                z_s(b);
                led.stage(b, "copy: SELECT V^dagger");
                if p.swap {
                    majorana_spin0(b, r, neg, id);
                } else {
                    majorana(b, r, neg, id);
                }
                led.stage(b, "copy: SELECT Majorana and controls");
                rotate(b, c, false);
                led.stage(b, "copy: SELECT V");
            } else {
                led.stage(b, "copy: SELECT V^dagger");
            }
        } else {
            rotate(b, c, false);
            led.stage(b, "copy: SELECT V");
        }
    }
    if let Some(g) = sel {
        b.spin_swap(g);
        spin_unselect(b, r, g);
        led.stage(b, "copy: SELECT controlled spin swap F^s");
    }

    // 4. RPREP^dagger.
    let last = states.len() - 1;
    let angles = |v: u64| word_of(v, last);
    erase_lookup_keep(b, &idx, e, r.angle_q, &angles, erase_split(e));
    if t.dense {
        sub_from(b, &bsel, &r.hi);
    } else {
        for (&o, &q) in r.lo.iter().zip(&bsel) {
            b.cx(o, q);
        }
    }
    led.stage(b, "copy: RPREP^dagger angle erasure");

    // 5. Inner PREPARE^dagger.
    choose_alt(b, lt, &main, alt);
    super::erase_lt(b, &r.draw, keep, lt, p.tw.gated);
    for (&i, &q) in r.index.iter().zip(&bsel) {
        b.cx(i, q);
    }
    bsel.into_iter().for_each(|q| b.free(q));
    led.stage(b, "copy: inner alt swap and keep test undone");
    qroam::erase(b, &index, read, &words, erase_split(limit));
    b.x(r.pass);
    led.stage(b, "copy: inner alias read erased");
}

/// The outer item's one-body flags and the enables formed from them (from the outer item, or,
/// in `sa-pareto`'s lean layout, from the angle word).
#[derive(Clone, Copy)]
pub struct Flags {
    /// The outer item is a one-body generator (not gated by the control).
    pub is_ob: Qubit,
    /// `[e_r >= 0]` of a one-body generator.
    pub pos_e: Qubit,
    /// `control AND is_ob` and `control AND NOT is_ob`.
    pub en_ob: Qubit,
    pub en_sq: Qubit,
}

/// The lane qubits the Majorana reads besides [`Flags`]: the inner index's low bit `x`, both
/// spin bits and `pass`.
#[derive(Clone, Copy)]
pub struct Lane {
    pub x: Qubit,
    pub s0: Qubit,
    pub s1: Qubit,
    pub pass: Qubit,
}

impl InnerRegs<'_> {
    /// This copy's [`Flags`].
    #[must_use]
    pub fn flags(&self) -> Flags {
        Flags {
            is_ob: self.is_ob,
            pos_e: self.pos_e,
            en_ob: self.en_ob,
            en_sq: self.en_sq,
        }
    }

    /// This copy's [`Lane`].
    #[must_use]
    pub fn lane(&self) -> Lane {
        Lane {
            x: self.index[0],
            s0: self.s0,
            s1: self.s1,
            pass: self.pass,
        }
    }
}

/// A fresh qubit holding the lane's generator spin: `s1` for a one-body generator, `s0` for a
/// square, `s0 XOR (is_ob AND (s0 XOR s1))` (one Toffoli). Not gated by the control: where the
/// control is 0 the Majorana is off and `F^s V V^dagger F^-s = I` whatever `s` is.
pub fn spin_select(b: &mut Builder, r: &InnerRegs<'_>) -> Qubit {
    spin_select_on(b, r.s0, r.s1, r.is_ob)
}

/// [`spin_select`] on explicit qubits.
pub fn spin_select_on(b: &mut Builder, s0: Qubit, s1: Qubit, is_ob: Qubit) -> Qubit {
    let g = b.alloc();
    b.cx(s0, s1);
    b.ccx(is_ob, s1, g);
    b.cx(s0, s1);
    b.cx(s0, g);
    g
}

/// Erases [`spin_select`]'s qubit by measurement.
pub fn spin_unselect(b: &mut Builder, r: &InnerRegs<'_>, g: Qubit) {
    spin_unselect_on(b, r.s0, r.s1, r.is_ob, g);
}

/// [`spin_unselect`] on explicit qubits.
pub fn spin_unselect_on(b: &mut Builder, s0: Qubit, s1: Qubit, is_ob: Qubit, g: Qubit) {
    b.cx(s0, g);
    b.cx(s0, s1);
    erase_and(b, is_ob, s1, g, false);
    b.cx(s0, s1);
}

/// The controlled Majorana on spin-0 mode 0 (system qubit 0), for the spin-swap SELECT.
pub fn majorana_spin0(b: &mut Builder, r: &InnerRegs<'_>, neg: Qubit, id: Qubit) {
    majorana_spin0_on(b, r.lane(), r.flags(), neg, id);
}

/// [`majorana_spin0`] on explicit qubits.
pub fn majorana_spin0_on(b: &mut Builder, l: Lane, fl: Flags, neg: Qubit, id: Qubit) {
    let z0 = b.system(0);
    // Square: sign, and Z on the pivot where en_sq AND NOT id.
    b.cz(fl.en_sq, neg);
    let pz = b.alloc();
    b.x(id);
    b.ccx(fl.en_sq, id, pz);
    b.x(id);
    b.cz(pz, z0);
    erase_and(b, fl.en_sq, id, pz, true);
    // One-body: X (x = 0) or Y = i X Z (x = 1) on the pivot; the phases as in `majorana`.
    let x = l.x;
    let ex = b.alloc();
    b.ccx(fl.en_ob, x, ex);
    b.cz(ex, z0);
    b.cx(fl.en_ob, z0);
    b.cz(ex, fl.pos_e);
    b.cz(ex, l.pass);
    erase_and(b, fl.en_ob, x, ex, false);
}

/// The controlled Majorana of both generator kinds, between `V^dagger` and `V`, when every
/// network runs on both spins: the pivot is mode `(0, s)`, system qubit `s`.
pub fn majorana(b: &mut Builder, r: &InnerRegs<'_>, neg: Qubit, id: Qubit) {
    majorana_on(b, r.lane(), r.flags(), neg, id);
}

/// [`majorana`] on explicit qubits.
pub fn majorana_on(b: &mut Builder, l: Lane, fl: Flags, neg: Qubit, id: Qubit) {
    let (z0, z1) = (b.system(0), b.system(1));
    // Square: sign, then Z on system qubit s0 where en_sq AND NOT id.
    b.cz(fl.en_sq, neg);
    let pz = b.alloc();
    b.x(id);
    b.ccx(fl.en_sq, id, pz);
    b.x(id);
    let pz1 = b.alloc();
    b.ccx(pz, l.s0, pz1);
    b.cx(pz1, pz);
    b.cz(pz, z0);
    b.cz(pz1, z1);
    b.cx(pz1, pz);
    erase_and(b, pz, l.s0, pz1, false);
    erase_and(b, fl.en_sq, id, pz, true);
    // One-body: gamma(u, s1, x) on mode (0, s1) = system qubit s1. x = inner index bit 0.
    let x = l.x;
    let e1 = b.alloc();
    b.ccx(fl.en_ob, l.s1, e1);
    let ex = b.alloc();
    b.ccx(fl.en_ob, x, ex);
    let ex1 = b.alloc();
    b.ccx(ex, l.s1, ex1);
    // Z part of Y = i X Z on the pivot qubit (x = 1), then the Jordan-Wigner string (s1 = 1),
    // then X on the pivot.
    b.cx(ex1, ex);
    b.cz(ex, z0);
    b.cz(ex1, z1);
    b.cx(ex1, ex);
    b.cz(e1, z0);
    b.cx(e1, fl.en_ob);
    b.cx(fl.en_ob, z0);
    b.cx(e1, fl.en_ob);
    b.cx(e1, z1);
    // Phase on x = 1: i (from Y) times +-i (M_1 = +-i gamma_1) is -1 for e_r >= 0 and +1
    // otherwise; the second copy's M^dagger adds -1.
    b.cz(ex, fl.pos_e);
    b.cz(ex, l.pass);
    erase_and(b, ex, l.s1, ex1, false);
    erase_and(b, fl.en_ob, x, ex, false);
    erase_and(b, fl.en_ob, l.s1, e1, false);
}
