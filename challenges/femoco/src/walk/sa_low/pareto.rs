//! `sa-pareto`: Low et al.'s step with lean gadgets, kept comparator carries, a dropped outer alt
//! slot and chunked angle loads (README section 5; `Params::lean`, `carries`,
//! `drop_alt`, `chunks`). Selected by `Params::pareto`. Its angle streaming is its own
//! (`2C - 1` ragged loads per copy, `shared::angles`'s schedule, the flag bits riding with the
//! first load), not `sa-lowq`'s (`inner.rs`), which streams through a dense or padded index with
//! XOR transitions; both are kept, each behind its own architecture.
//!
//! One copy, as `inner.rs` describes, with these differences:
//!
//! - the inner word drops the `id` flags (`id = [b = B]` is computed from `b` after the swap,
//!   `k_i - 1` Toffolis), the one-body flags `is_ob | pos_e` come with the angle word and
//!   `en_ob`, `en_sq` are formed in the copy, and the RPREP iteration skips left-only ANDs
//!   (`SaTables::leaf`) (`lean`);
//! - the comparisons can keep their carries for a free erasure (`carries`: bit 1 outer, bit 2
//!   inner);
//! - the angles can be held `g` rotations at a time (`chunks`).
use super::inner::{
    choose_alt, erase_split, majorana_on, majorana_spin0_on, spin_select_on, spin_unselect_on,
    Flags, Lane,
};
use super::ledger::Ledger;
use super::narrow;
use super::qroam;
use super::tables::SaTables;
use super::Params;
use crate::circuit::{Bit, Builder, Qubit, Reg, SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE};
use crate::lanemap::sa_nested::SaNestedMap;
use crate::spec::sa::SaSpec;
use crate::walk::common::arith::{
    all_ones, erase_all_ones, erase_less_than_keep, less_than, less_than_keep, unless_than,
};
use crate::walk::common::arith_gated::unless_than_gated;
use crate::walk::common::lookup::erase_lookup_keep;
use crate::walk::common::unary::erase_and;
use crate::walk::common::unary::iterate;
use crate::walk::shared::lookup::{measure, put, set_bits, word, xor, Word};

/// The narrowing gadgets (`FEMOCO_SA_NARROW`, one letter each, some with a block count;
/// `README.md` here). Every one is off by default, so the plain `sa-pareto`
/// circuits emit the same ops. All need the lean layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Narrow {
    /// `e`: `en_ob`, `en_sq` are formed right before the Majorana and erased right after it (0
    /// Toffolis each way instead of 1), and the spin-select qubit is erased after `F^-s` and made
    /// again before `F^s` (+1 Toffoli per copy): none of the three is live across the chunk
    /// transitions.
    pub flags: bool,
    /// `k<a>`: the outer keep register is measured right after the comparison; the comparison is
    /// erased by the outcome-gated re-read (`narrow::gated_lt_erase`, `2^a` blocks, default
    /// `a = 2`), whose outcomes join the outer read's final fixup. `-mu_o` qubits across the
    /// step. Needs `drop_alt` and the plain outer comparison.
    pub outer_keep: Option<usize>,
    /// `r<a>`: the same for the inner keep register in each copy (`-mu_i` qubits across SELECT;
    /// default `a = 2`). Needs the plain inner comparison.
    pub inner_keep: Option<usize>,
    /// `g`: every plain keep comparison still erased with its keep register held is erased by
    /// the outcome-gated recompute (`common::arith_gated`, brief 1's helper): `(mu - 1) / 2`
    /// expected Toffolis instead of `mu - 1`, no qubits.
    pub gated: bool,
    /// `c`: the compact outer data `q | hi` (`SaTables::with_pareto`): no `lo` field (`-k_i`
    /// qubits across the step); each copy XORs `is_ob AND q_low` into `b` and out again
    /// ([`lo_into`], `2 (q_b - kb - 1 + k_i)` Toffolis per copy).
    pub compact: bool,
    /// `a`: with `r`, the inner alias read's unchosen item is measured right after the swap
    /// (`-(k_i + 1)` qubits across SELECT); the item register is measured at PREPARE^dagger and
    /// one fixup over `[lt] ++ x` cancels both, with the read's junk and keep
    /// ([`inner_unprepare_dropped`]).
    pub inner_alt: bool,
    /// `p`: the copy's pass flag is a classical bit toggled at the end of each copy (the two
    /// copies are the same ops, so it reads 0, then 1) instead of a qubit: `-1` qubit, no
    /// Toffolis. Needs `e`.
    pub classical_pass: bool,
    /// `j`: the Majorana stage holds `id` without its AND chain and one enable at a time
    /// ([`majorana_narrow`]): `-4` qubits there for `(k_i - 2) / 2 + 1` expected Toffolis per
    /// copy. Needs `e`.
    pub majorana_cut: bool,
    /// `o`: with `k` and `drop_alt`, the outer comparison bit is measured right after the item
    /// is chosen; UNPREPARE recomputes it from one re-read of `keep | own ^ alt`
    /// ([`outer_unprepare_lt`]) instead of the controlled fixup and the gated re-read: `-1`
    /// qubit across the step.
    pub outer_lt: bool,
    /// `q<a>`: the outer item's `q` field is measured after the RPREP read and read again
    /// (`2^a` blocks, default 2) before RPREP^dagger, in each copy ([`restore_q`]): `-q_b`
    /// qubits across SELECT. Needs `drop_alt`; not with `o` (it reads `lt`).
    pub release_q: Option<usize>,
    /// `h<n>`: at most `2^n` one-hot qubits in the fixups these gadgets add (default 6)
    /// and, when given, in the plain inner erasure's fixup too (`cap_erase`).
    pub hot_cap: usize,
    pub cap_erase: bool,
}

impl Narrow {
    /// Every gadget off.
    pub const OFF: Self = Self {
        flags: false,
        outer_keep: None,
        inner_keep: None,
        gated: false,
        compact: false,
        inner_alt: false,
        classical_pass: false,
        majorana_cut: false,
        outer_lt: false,
        release_q: None,
        hot_cap: 6,
        cap_erase: false,
    };

    /// Parses a letter string such as `"ek2r3"` (a letter, then an optional block count).
    ///
    /// # Panics
    /// On an unknown letter.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        let mut n = Self::OFF;
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            let mut digits = String::new();
            while let Some(d) = it.peek().filter(|d| d.is_ascii_digit()) {
                digits.push(*d);
                it.next();
            }
            let num = digits.parse::<usize>().ok();
            match c {
                'e' => n.flags = true,
                'g' => n.gated = true,
                'c' => n.compact = true,
                'a' => n.inner_alt = true,
                'p' => n.classical_pass = true,
                'j' => n.majorana_cut = true,
                'o' => n.outer_lt = true,
                'q' => n.release_q = Some(num.unwrap_or(2)),
                'h' => {
                    n.hot_cap = num.unwrap_or(6);
                    n.cap_erase = true;
                }
                'k' => n.outer_keep = Some(num.unwrap_or(2)),
                'r' => n.inner_keep = Some(num.unwrap_or(2)),
                _ => panic!("FEMOCO_SA_NARROW: unknown gadget {c:?} in {s:?}"),
            }
        }
        n
    }

    /// Whether any gadget is on.
    #[must_use]
    pub fn any(&self) -> bool {
        *self != Self::OFF
    }
}

/// What a copy reads, all prepared outside it.
pub struct Regs<'a> {
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
    /// The one-body flags from the outer item; `None` in the lean layout, where they come with
    /// the angle word.
    pub flags: Option<Flags>,
    /// 0 in the first copy, 1 in the second (each copy toggles it at its end).
    pub pass: Qubit,
    /// `p`: the same, as a classical bit instead of `pass` (which is then never touched).
    pub pass_bit: Option<Bit>,
    /// The angle register (slots, then flag bits) and one register per slot.
    pub angle_q: &'a [Qubit],
    pub angle_regs: &'a [Reg],
    /// `q`: what re-reading the outer `q` field needs.
    pub outer_q: Option<OuterQ>,
}

/// `q`: the outer lookup index, the outer comparison bit and where `q` sits in an outer item.
#[derive(Clone, Debug)]
pub struct OuterQ {
    pub index: Vec<Qubit>,
    pub lt: Qubit,
    /// `q`'s bits within one item's data, and the item's width `d`, keep width `mu`.
    pub field: std::ops::Range<usize>,
    pub d: usize,
    pub mu: usize,
    /// log2 of the re-read's block count.
    pub a: usize,
}

/// A keep comparison `[draw < keep]` and what its erasure needs.
pub enum Cmp {
    /// The result alone (erased by recomputing the lower carries, `n - 1` Toffolis).
    Plain(Qubit),
    /// Every carry kept (erased by measurement, no Toffolis).
    Kept(Vec<Qubit>),
}

impl Cmp {
    /// Computes `[a < bb]`, keeping the carries when `keep`.
    pub fn new(b: &mut Builder, a: &[Qubit], bb: &[Qubit], keep: bool) -> Self {
        if keep {
            Cmp::Kept(less_than_keep(b, a, bb))
        } else {
            Cmp::Plain(less_than(b, a, bb))
        }
    }

    /// The result qubit.
    #[must_use]
    pub fn lt(&self) -> Qubit {
        match self {
            Cmp::Plain(q) => *q,
            Cmp::Kept(cs) => *cs.last().expect("a carry"),
        }
    }

    /// Erases the comparison (on unchanged registers).
    pub fn erase(self, b: &mut Builder, a: &[Qubit], bb: &[Qubit]) {
        self.erase_by(b, a, bb, false);
    }

    /// [`Cmp::erase`]; with `gated` a plain comparison is erased by the outcome-gated
    /// recompute (`common::arith_gated::unless_than_gated`: `(n - 1) / 2` expected Toffolis).
    pub fn erase_by(self, b: &mut Builder, a: &[Qubit], bb: &[Qubit], gated: bool) {
        match self {
            Cmp::Plain(lt) if gated => unless_than_gated(b, a, bb, lt),
            Cmp::Plain(lt) => unless_than(b, a, bb, lt),
            Cmp::Kept(cs) => erase_less_than_keep(b, a, bb, &cs),
        }
    }
}

/// Emits the `sa-pareto` step for a given lane map (`map.uniform_bits()` must already be
/// declared) and returns the per-stage ledger.
#[allow(clippy::too_many_lines)]
pub fn emit(spec: &SaSpec, map: &SaNestedMap, b: &mut Builder, p: Params) -> Ledger {
    assert!(p.swap || !p.lean, "the lean layout needs the spin swap");
    assert!(
        !p.outer_erase && !p.dense,
        "sa-lowq's outer erase and dense index are not part of sa-pareto"
    );
    let t = SaTables::with_pareto(spec, map, p.lean, p.chunks, p.narrow.compact);
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
    let angle_q = b.alloc_n(t.angle_bits());
    let angle_regs: Vec<Reg> = (0..t.g)
        .map(|s| b.register(&angle_q[t.slot_at[s]..t.slot_at[s] + t.slot_w[s]]))
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
    let cmp = Cmp::new(b, &draw_o, &keep, p.carries & 1 != 0);
    let lt = cmp.lt();
    let nw = p.narrow;
    assert!(
        !nw.any() || p.lean,
        "the stream-narrow gadgets need the lean layout"
    );
    // `k`: the keep register goes as soon as the comparison is made.
    let keep_bits = nw.outer_keep.map(|_| {
        assert!(
            p.drop_alt && p.carries & 1 == 0,
            "the outer keep release needs drop_alt and the plain outer comparison"
        );
        measure(b, keep.clone())
    });
    choose_alt(b, lt, &main, &alt);
    let alt_bits = p.drop_alt.then(|| measure(b, alt.clone()));
    // `o`: lt goes too; UNPREPARE recomputes it from a re-read of `keep | own ^ alt`.
    let lt_bit = nw.outer_lt.then(|| {
        assert!(
            p.drop_alt && nw.outer_keep.is_some(),
            "o needs drop_alt and k"
        );
        b.hmr(lt)
    });
    let (lo_at, hi_at, q_at, ob_at, pos_at) = t.outer_fields();
    let control = b.control();
    let flags = (!p.lean).then(|| {
        let is_ob = main[ob_at];
        let en_ob = b.alloc();
        b.ccx(control, is_ob, en_ob);
        let en_sq = b.alloc();
        b.cx(control, en_sq);
        b.cx(en_ob, en_sq);
        Flags {
            is_ob,
            pos_e: main[pos_at],
            en_ob,
            en_sq,
        }
    });
    let pass = b.alloc();
    // `p`: both copies are the same ops, so a classical bit toggled at the end of each copy
    // reads 0 in the first and 1 in the second, as the qubit does.
    let pass_bit = nw.classical_pass.then(|| narrow::zero_bit(b));
    led.stage(b, "outer: keep test, alt swap, enables");

    b.segment(SEG_SELECT);
    let q_end = if p.lean { main.len() } else { ob_at };
    // `c`: the compact outer data `q | hi` (no `lo`).
    let (lo_f, hi_f, q_f) = if t.compact {
        (0..0, t.q_b..t.q_b + t.h, 0..t.q_b)
    } else {
        (lo_at..hi_at, hi_at..q_at, q_at..q_end)
    };
    let regs = Regs {
        index: index_i,
        draw: draw_i,
        s0,
        s1,
        lo: main[lo_f].to_vec(),
        hi: main[hi_f].to_vec(),
        q: main[q_f.clone()].to_vec(),
        flags,
        pass,
        pass_bit,
        angle_q: &angle_q,
        angle_regs: &angle_regs,
        outer_q: nw.release_q.map(|a| {
            assert!(
                p.drop_alt && !nw.outer_lt,
                "q needs drop_alt and the outer lt (not o)"
            );
            OuterQ {
                index: index_o.clone(),
                lt,
                field: q_f.clone(),
                d,
                mu: mu_o as usize,
                a,
            }
        }),
    };
    b.nested_inner(inner_reg, |b| copy(b, &t, &regs, p, &mut led));
    led.start(b);

    b.segment(SEG_UNPREPARE);
    b.free(pass);
    if let Some(f) = flags {
        b.cx(f.en_ob, f.en_sq);
        b.cx(control, f.en_sq);
        b.free(f.en_sq);
        erase_and(b, control, f.is_ob, f.en_ob, false);
    }
    if let Some(m_a) = alt_bits {
        // The chosen slot holds own(i) where lt else alt(i); the measured slot the other. Their
        // outcomes left (-1)^(m_a.own ^ m_m.alt) (folded into the final fixup, whose word is
        // keep | own | alt) times (-1)^(lt (m_a ^ m_m).(own ^ alt)), cancelled here while lt
        // is live.
        let m_m = measure(b, main.clone());
        let bits: Vec<Bit> = m_a.iter().chain(&m_m).copied().collect();
        let mu = mu_o as usize;
        if let Some(m_l) = lt_bit {
            let a2 = nw.outer_keep.expect("checked").min(index_o.len());
            outer_unprepare_lt(
                b,
                (&index_o, buckets, &draw_o),
                (read, &words, p.outer_a.min(index_o.len()), mu, d),
                (keep_bits.clone().expect("k measured keep"), m_a, m_m, m_l),
                a2,
                nw.gated,
            );
            led.stage(
                b,
                "outer^-1: keep and own ^ alt re-read, lt recomputed, alias reads erased",
            );
            angle_q.into_iter().for_each(|q| b.free(q));
            return led;
        }
        let diff = |i: u64| {
            let w = words(i);
            let (mut own, mut alt) = (word(d), word(d));
            for j in 0..d {
                let bit = |x: usize| w[x / 64] >> (x % 64) & 1;
                put(&mut own, j, 1, bit(mu + j));
                put(&mut alt, j, 1, bit(mu + d + j));
            }
            let x = xor(&own, &alt);
            let mut out = word(2 * d);
            for j in 0..d {
                let v = x[j / 64] >> (j % 64) & 1;
                put(&mut out, j, 1, v);
                put(&mut out, d + j, 1, v);
            }
            out
        };
        qroam::phase_fixup_ctl(b, lt, &index_o, buckets, &bits, &diff, erase_split(buckets));
        let Some(a2) = nw.outer_keep else {
            cmp.erase_by(b, &draw_o, &keep, nw.gated);
            led.stage(b, "outer^-1: controlled fixup, keep test, enables");
            let mut out = measure(b, keep.clone());
            out.extend(m_a);
            out.extend(m_m);
            qroam::erase_measured(b, &index_o, read, out, &words, erase_split(buckets));
            led.stage(b, "outer^-1: alias read erased");
            angle_q.into_iter().for_each(|q| b.free(q));
            return led;
        };
        led.stage(b, "outer^-1: controlled fixup, enables");
        let keep_word = |i: u64| narrow::low_bits(&words(i), mu);
        let extra = narrow::gated_lt_erase(
            b,
            lt,
            &draw_o,
            &index_o,
            buckets,
            &keep_word,
            narrow::Reread { a: a2, fix: None },
            narrow::Fault::None,
        );
        led.stage(b, "outer^-1: gated keep re-read");
        let a1 = p.outer_a.min(index_o.len());
        let a2 = a2.min(index_o.len());
        let mut out = keep_bits.expect("measured with the comparison");
        out.extend(m_a);
        out.extend(m_m);
        let mut first = read.into_junk();
        first.extend(out);
        let w1 = mu + 2 * d;
        let c1 = |x: u64| narrow::read_content(x, a1, w1, buckets, &words);
        let c2 = |x: u64| narrow::read_content(x, a2, mu, buckets, &keep_word);
        narrow::fixup_parts(
            b,
            &index_o,
            buckets,
            &[(&first, &c1), (&extra, &c2)],
            qroam::Split {
                h: erase_split(buckets),
                hot: None,
            },
        );
    } else {
        choose_alt(b, lt, &main, &alt);
        cmp.erase_by(b, &draw_o, &keep, nw.gated);
        led.stage(b, "outer^-1: alt swap, keep test, enables");
        qroam::erase(b, &index_o, read, &words, erase_split(buckets));
    }
    led.stage(b, "outer^-1: alias read erased");
    angle_q.into_iter().for_each(|q| b.free(q));
    led
}

/// Unary iteration over `index` for values `0..limit` when every lane with `ctl = 1` holds an
/// index below `limit` (or does not care which leaf it reaches, `SaTables::leaf`): a node with
/// only a left child passes its control down instead of ANDing it with the negated index bit,
/// so a controlled iteration costs `limit - 1` Toffolis.
fn iterate_in_range(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    limit: u64,
    leaf: &mut dyn FnMut(&mut Builder, u64, Qubit),
) {
    node_in_range(b, ctl, index, 0, limit, leaf);
}

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
        // Left child only: the bit is 0 on every lane that matters.
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

/// XORs `data(idx)` into `out` for network index `idx = lo | hi << k_i` by a ragged unary
/// iteration: over `hi` (`R` square rows, then the one-body rows), and inside each row over `lo`
/// up to that row's length (`B` for a square row, so the padding is never visited). Low et
/// al.'s `N + R B` RPREP iteration, instead of `R 2^k_i + N` over the padded index. The lean
/// layout's iteration skips left-only ANDs (the identity item then reaches leaf
/// `SaTables::leaf(idx)` instead of none).
fn ragged_read(
    b: &mut Builder,
    t: &SaTables<'_>,
    lo: &[Qubit],
    hi: &[Qubit],
    out: &[Qubit],
    data: &dyn Fn(u64) -> Word,
) {
    let row = 1u64 << t.k_i;
    let lean = t.lean;
    let mut leaf = |b: &mut Builder, hv: u64, flag: Qubit| {
        let len = t.row_len(hv);
        let mut inner = |b: &mut Builder, v: u64, f: Qubit| {
            for j in set_bits(&data(hv * row + v), out.len()) {
                b.cx(f, out[j]);
            }
        };
        if lean {
            iterate_in_range(b, Some(flag), lo, len, &mut inner);
        } else {
            iterate(b, Some(flag), lo, len, &mut inner);
        }
    };
    if lean {
        iterate_in_range(b, None, hi, t.rows(), &mut leaf);
    } else {
        iterate(b, None, hi, t.rows(), &mut leaf);
    }
}

/// `[x = v]` for a constant `v` into a fresh qubit (`|x| - 1` Toffolis), with its AND chain.
fn equals_const(b: &mut Builder, x: &[Qubit], v: u64) -> (Qubit, Vec<(Qubit, Qubit, Qubit)>) {
    flip_zeros(b, x, v);
    let (q, chain) = all_ones(b, x);
    flip_zeros(b, x, v);
    (q, chain)
}

fn unequals_const(b: &mut Builder, x: &[Qubit], v: u64, chain: Vec<(Qubit, Qubit, Qubit)>) {
    flip_zeros(b, x, v);
    erase_all_ones(b, chain);
    flip_zeros(b, x, v);
}

fn flip_zeros(b: &mut Builder, x: &[Qubit], v: u64) {
    for (j, &q) in x.iter().enumerate() {
        if v >> j & 1 == 0 {
            b.x(q);
        }
    }
}

/// `b ^= lo`, which makes the RPREP index's low field (self-inverse). In the compact outer data
/// (`c`) a one-body item's `lo` is the low `k_i` bits of its `q = ones | r`, and `is_ob` is the
/// AND of `q`'s `ones` bits (no square reaches them): `q_b - kb - 1` Toffolis for `is_ob`
/// (its chain erased by measurement at once, with Clifford fixups) and `k_i` for the
/// controlled XOR.
fn lo_into(b: &mut Builder, t: &SaTables<'_>, r: &Regs<'_>, bsel: &[Qubit]) {
    if !t.compact {
        for (&o, &q) in r.lo.iter().zip(bsel) {
            b.cx(o, q);
        }
        return;
    }
    let ones = &r.q[t.kb..t.q_b];
    let (is_ob, chain) = if ones.len() == 1 {
        (ones[0], None)
    } else {
        let (q, c) = all_ones(b, ones);
        (q, Some(c))
    };
    for (&q, &s) in r.q[..t.k_i].iter().zip(bsel) {
        b.ccx(is_ob, q, s);
    }
    if let Some(c) = chain {
        erase_all_ones(b, c);
    }
}

/// Outer UNPREPARE with `keep`, the unchosen slot and `lt` all measured early (`o`): the
/// outcomes left `lt . (m_l ^ (m_a ^ m_m) . (own ^ alt))` besides index-only phases. A read of
/// `keep | own ^ alt` (`2^a2` blocks) gives `lt` again (a comparison, erased by the gated or
/// plain recompute) and the bit `g = m_l ^ (m_a ^ m_m) . (own ^ alt)` (classically controlled
/// CNOTs); `CZ(lt, g)` cancels it. One fixup then cancels both reads.
fn outer_unprepare_lt(
    b: &mut Builder,
    (index_o, buckets, draw_o): (&[Qubit], u64, &[Qubit]),
    (read, words, a1, mu, d): (qroam::Read, &dyn Fn(u64) -> Word, usize, usize, usize),
    (m_k, m_a, m_m, m_l): (Vec<Bit>, Vec<Bit>, Vec<Bit>, Bit),
    a2: usize,
    gated: bool,
) {
    let data2 = |i: u64| -> Word {
        let w = words(i);
        let bit = |x: usize| w[x / 64] >> (x % 64) & 1;
        let mut out = word(mu + d);
        for j in 0..mu {
            put(&mut out, j, 1, bit(j));
        }
        for j in 0..d {
            put(&mut out, mu + j, 1, bit(mu + j) ^ bit(mu + d + j));
        }
        out
    };
    let rd = qroam::load(b, index_o, buckets, mu + d, &data2, a2);
    let (keep2, delta) = rd.out.split_at(mu);
    let c: Vec<Bit> = m_a
        .iter()
        .zip(&m_m)
        .map(|(&x, &y)| {
            let bit = narrow::zero_bit(b);
            for src in [x, y] {
                let mut op = crate::circuit::Op::new(crate::circuit::OperationType::BitInvert);
                op.c_target = bit.0;
                op.c_condition = src.0;
                b.emit(op);
            }
            bit
        })
        .collect();
    let g = b.alloc();
    let make_g = |b: &mut Builder| {
        b.x_if(g, m_l);
        for (&q, &cj) in delta.iter().zip(&c) {
            b.cx_if(q, g, cj);
        }
    };
    make_g(b);
    let lt2 = less_than(b, draw_o, keep2);
    b.cz(lt2, g);
    if gated {
        unless_than_gated(b, draw_o, keep2, lt2);
    } else {
        unless_than(b, draw_o, keep2, lt2);
    }
    make_g(b);
    b.free(g);
    let out2 = measure(b, rd.out.clone());
    let mut second = rd.into_junk();
    second.extend(out2);
    let mut first = read.into_junk();
    first.extend(m_k);
    first.extend(m_a);
    first.extend(m_m);
    let c1 = |x: u64| narrow::read_content(x, a1, mu + 2 * d, buckets, words);
    let c2 = |x: u64| narrow::read_content(x, a2, mu + d, buckets, &data2);
    narrow::fixup_parts(
        b,
        index_o,
        buckets,
        &[(&first, &c1), (&second, &c2)],
        qroam::Split {
            h: erase_split(buckets),
            hot: None,
        },
    );
}

/// `q`: puts the outer item's `q` field back into `r.q` (measured, so `|0>`, since the RPREP
/// read). The measurement left `(-1)^(m_q . (lt ? own.q : alt.q))`. A read of `own.q | alt.q`
/// over the outer index, a swap by `lt` and a measurement of the unchosen half leave
/// `(-1)^(m_o . (lt ? alt.q : own.q))` and the read's junk; one fixup over `[lt] ++ x_o`
/// cancels all of them inside the copy (the two copies share their classical bits).
fn restore_q(
    b: &mut Builder,
    t: &SaTables<'_>,
    r: &Regs<'_>,
    oq: &OuterQ,
    m_q: Vec<Bit>,
    cap: usize,
) {
    let nq = oq.field.len();
    let buckets = 1u64 << oq.index.len();
    let field = |i: u64, alt: bool| -> u64 {
        let w = t.outer_word(i);
        let at = oq.mu + if alt { oq.d } else { 0 } + oq.field.start;
        (0..nq).fold(0u64, |v, j| {
            v | (w[(at + j) / 64] >> ((at + j) % 64) & 1) << j
        })
    };
    let data = |i: u64| -> Word {
        let mut out = word(2 * nq);
        put(&mut out, 0, nq, field(i, false));
        put(&mut out, nq, nq, field(i, true));
        out
    };
    let a = oq.a.min(oq.index.len());
    let rd = qroam::load(b, &oq.index, buckets, 2 * nq, &data, a);
    let (own, alt) = rd.out.split_at(nq);
    // own <- lt ? own.q : alt.q, alt <- the other.
    choose_alt(b, oq.lt, own, alt);
    for (&x, &q) in own.iter().zip(&r.q) {
        b.swap(x, q);
    }
    let m_o = measure(b, alt.to_vec());
    let mut bits = Vec::new();
    let out_slots = rd.out.clone();
    let junk = rd.into_junk();
    let junk_n = junk.len();
    bits.extend(junk);
    bits.extend(m_q);
    bits.extend(m_o);
    for q in out_slots.into_iter().take(nq) {
        b.free(q);
    }
    let all = |xl: u64| -> Word {
        let (l, x) = (xl & 1, xl >> 1);
        let c = narrow::read_content(x, a, 2 * nq, buckets, &data);
        let mut out = word(junk_n + 2 * nq);
        for j in set_bits(&c, junk_n) {
            out[j / 64] |= 1 << (j % 64);
        }
        let (o, al) = (field(x, false), field(x, true));
        put(&mut out, junk_n, nq, if l == 1 { o } else { al });
        put(&mut out, junk_n + nq, nq, if l == 1 { al } else { o });
        out
    };
    let idx2: Vec<Qubit> = std::iter::once(oq.lt)
        .chain(oq.index.iter().copied())
        .collect();
    let h = narrow::fixup_bits(idx2.len(), 2 * buckets, cap);
    qroam::fixup(b, &idx2, 2 * buckets, &bits, &all, h);
}

/// Toggles the copy's pass flag (qubit, or classical bit with `p`).
fn toggle_pass(b: &mut Builder, r: &Regs<'_>) {
    match r.pass_bit {
        Some(bit) => {
            let mut op = crate::circuit::Op::new(crate::circuit::OperationType::BitInvert);
            op.c_target = bit.0;
            b.emit(op);
        }
        None => b.x(r.pass),
    }
}

/// The controlled Majorana on spin-0 mode 0 (`inner::majorana_spin0_on`, same operator) for the
/// lean layout with `e`: the enables are made here. With `cut` (`j`) the square part runs
/// under `en_sq = control AND NOT is_ob` alone and the one-body part under `en_ob` alone (one
/// Toffoli each), and `id = [b = B]` keeps no chain: its intermediate ANDs are measured at once
/// (Clifford fixups), and `id` is erased by an X measurement whose outcome gates a recompute
/// of its phase (`(k_i - 2) / 2` expected Toffolis). The pass flag is the classical bit when
/// `p` is on.
fn majorana_narrow(
    b: &mut Builder,
    t: &SaTables<'_>,
    r: &Regs<'_>,
    bsel: &[Qubit],
    neg: Qubit,
    cut: bool,
) {
    let (is_ob, pos_e) = (r.angle_q[t.slot_bits()], r.angle_q[t.slot_bits() + 1]);
    let control = b.control();
    let z0 = b.system(0);
    let bv = t.spec.b as u64;
    // id = [bsel = B].
    flip_zeros(b, bsel, bv);
    let (id, chain) = all_ones(b, bsel);
    let chain = if cut {
        // Keep only id: erase the chain below it, top first (each needs its own inputs only).
        let mut ch = chain;
        let top = ch.pop();
        erase_all_ones(b, ch);
        top.map(|t| vec![t]).unwrap_or_default()
    } else {
        chain
    };
    flip_zeros(b, bsel, bv);
    let (en_ob, en_sq) = if cut {
        let en_sq = b.alloc();
        b.x(is_ob);
        b.ccx(control, is_ob, en_sq);
        b.x(is_ob);
        (None, en_sq)
    } else {
        let en_ob = b.alloc();
        b.ccx(control, is_ob, en_ob);
        let en_sq = b.alloc();
        b.cx(control, en_sq);
        b.cx(en_ob, en_sq);
        (Some(en_ob), en_sq)
    };
    // Square: sign, and Z on the pivot where en_sq AND NOT id.
    b.cz(en_sq, neg);
    let pz = b.alloc();
    b.x(id);
    b.ccx(en_sq, id, pz);
    b.x(id);
    b.cz(pz, z0);
    erase_and(b, en_sq, id, pz, true);
    let en_ob = match en_ob {
        Some(e) => {
            b.cx(e, en_sq);
            b.cx(control, en_sq);
            b.free(en_sq);
            e
        }
        None => {
            erase_and(b, control, is_ob, en_sq, true);
            let e = b.alloc();
            b.ccx(control, is_ob, e);
            e
        }
    };
    // One-body: X (x = 0) or Y = i X Z (x = 1) on the pivot.
    let x = r.index[0];
    let ex = b.alloc();
    b.ccx(en_ob, x, ex);
    b.cz(ex, z0);
    b.cx(en_ob, z0);
    b.cz(ex, pos_e);
    match r.pass_bit {
        Some(bit) => b.z_if(ex, bit),
        None => b.cz(ex, r.pass),
    }
    erase_and(b, en_ob, x, ex, false);
    erase_and(b, control, is_ob, en_ob, false);
    // id and its chain.
    flip_zeros(b, bsel, bv);
    if cut && !chain.is_empty() {
        let m = b.hmr(id);
        let mut pool = narrow::Pool::default();
        b.push_condition(m);
        narrow::and_phase(b, bsel, &mut pool);
        b.pop_condition();
        pool.close(b);
    } else {
        erase_all_ones(b, chain);
    }
    flip_zeros(b, bsel, bv);
}

/// Inner PREPARE^dagger with the unchosen item already measured (`a`, with `r`). The item
/// register (`b` in `bsel`, then the own flags slot) holds `lt ? O : A` and the measured alt
/// register held `lt ? A : O`, with `O = (i, own flags)`, `A = (alt b, alt flags)`. Measuring the
/// item leaves `m_a . (lt ? A : O) ^ m_m . (lt ? O : A)`. Its `i` parts are Cliffords on the index
/// bits (`Z^(m_a)` on `i`, and `CZ(lt, i)` for both outcomes); the rest is a table over
/// `(lt, x)`, cancelled together with the read's junk and keep outcomes by one fixup over the
/// index `[lt] ++ x`. Then `lt` is erased by the gated re-read, whose own fixup runs inside its
/// block.
#[allow(clippy::too_many_arguments)]
fn inner_unprepare_dropped(
    b: &mut Builder,
    t: &SaTables<'_>,
    r: &Regs<'_>,
    p: Params,
    (read, words, index, limit): (qroam::Read, &dyn Fn(u64) -> Word, Vec<Qubit>, u64),
    main: Vec<Qubit>,
    (keep_bits, m_a): (Option<Vec<Bit>>, Vec<Bit>),
    lt: Qubit,
) {
    let k_i = t.k_i;
    let mu = r.draw.len();
    let fw = t.flag_bits();
    let m_m = measure(b, main);
    // The i parts, by Cliffords.
    for j in 0..k_i {
        let i = r.index[j];
        b.z_if(i, m_a[j]);
        b.cz_if(lt, i, m_a[j]);
        b.cz_if(lt, i, m_m[j]);
    }
    let a1 = p.inner_a.min(k_i).min(index.len());
    let w1 = t.inner_word_bits();
    let full = narrow::read_full(index.len(), limit, a1);
    let mut first = read.into_junk();
    let junk_n = first.len();
    first.extend(keep_bits.expect("measured with the comparison"));
    let own = |x: u64| -> (u64, u64, u64) {
        // (own flags, alt b, alt flags) of word x
        let w = words(x);
        let get = |at: usize, n: usize| {
            (0..n).fold(0u64, |v, j| {
                v | (w[(at + j) / 64] >> ((at + j) % 64) & 1) << j
            })
        };
        (get(mu, fw), get(mu + fw, k_i), get(mu + fw + k_i, fw))
    };
    let slot = k_i + fw;
    let data = |xl: u64| -> Word {
        let (l, x) = (xl & 1, xl >> 1);
        let c1 = narrow::read_content(x, a1, w1, limit, words);
        let mut out = word(junk_n + mu + 2 * slot);
        for j in set_bits(&c1, junk_n + mu) {
            out[j / 64] |= 1 << (j % 64);
        }
        if x < limit {
            let (of, ab, af) = own(x);
            // alt register: lt ? A : O (O's b part is i: Cliffords above).
            let a_reg = if l == 1 { ab | af << k_i } else { of << k_i };
            // item register: lt ? O : A.
            let m_reg = if l == 1 { of << k_i } else { ab | af << k_i };
            put(&mut out, junk_n + mu, slot, a_reg);
            put(&mut out, junk_n + mu + slot, slot, m_reg);
        }
        out
    };
    let mut bits = first;
    bits.extend(m_a);
    bits.extend(m_m);
    let idx2: Vec<Qubit> = std::iter::once(lt).chain(index.iter().copied()).collect();
    let h = narrow::fixup_bits(idx2.len(), 2 * full, p.narrow.hot_cap);
    qroam::fixup(b, &idx2, 2 * full, &bits, &data, h);
    let a2 = p.narrow.inner_keep.expect("checked").min(index.len());
    let keep_word = |x: u64| narrow::low_bits(&words(x), mu);
    let full2 = narrow::read_full(index.len(), limit, a2);
    let h2 = (0..=index.len().min(p.narrow.hot_cap))
        .min_by_key(|&h| 2 * ((1u64 << h) - 1) + full2.div_ceil(1 << h))
        .unwrap_or(0);
    let _ = narrow::gated_lt_erase(
        b,
        lt,
        &r.draw,
        &index,
        limit,
        &keep_word,
        narrow::Reread {
            a: a2,
            fix: Some(h2),
        },
        narrow::Fault::None,
    );
}

/// Emits one inner copy.
#[allow(clippy::too_many_lines)]
fn copy(b: &mut Builder, t: &SaTables<'_>, r: &Regs<'_>, p: Params, led: &mut Ledger) {
    let k_i = t.k_i;
    let mu = r.draw.len();
    let fw = t.flag_bits();
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
    let own_flags = &out[mu..mu + fw];
    let alt = &out[mu + fw..];
    let bsel = b.alloc_n(k_i);
    for (&i, &q) in r.index.iter().zip(&bsel) {
        b.cx(i, q);
    }
    let main: Vec<Qubit> = [&bsel[..], own_flags].concat();
    let cmp = Cmp::new(b, &r.draw, keep, p.carries & 2 != 0);
    let nw = p.narrow;
    // `r`: the keep register goes as soon as the comparison is made.
    let keep_bits = nw.inner_keep.map(|_| {
        assert!(
            p.carries & 2 == 0,
            "the inner keep release needs the plain inner comparison"
        );
        measure(b, keep.to_vec())
    });
    choose_alt(b, cmp.lt(), &main, alt);
    let neg = main[k_i];
    // `a`: the unchosen item goes at once (its lt-dependent phase is cancelled at the end).
    let alt_bits = nw.inner_alt.then(|| {
        assert!(
            nw.inner_keep.is_some(),
            "the inner alt drop needs the inner keep release"
        );
        measure(b, alt.to_vec())
    });
    led.stage(b, "copy: inner keep test and alt swap");

    // 2. RPREP: idx = lo | hi; the last chunk (and the flags) first.
    lo_into(b, t, r, &bsel);
    let idx: Vec<Qubit> = [&bsel[..], &r.hi[..]].concat();
    let e = t.net_limit();
    let last = t.chunks - 1;
    let first = |v: u64| t.lean_word(v, last, true);
    ragged_read(b, t, &bsel, &r.hi, r.angle_q, &first);
    let make_flags = |b: &mut Builder| {
        let (is_ob, pos_e) = (r.angle_q[t.slot_bits()], r.angle_q[t.slot_bits() + 1]);
        let control = b.control();
        let en_ob = b.alloc();
        b.ccx(control, is_ob, en_ob);
        let en_sq = b.alloc();
        b.cx(control, en_sq);
        b.cx(en_ob, en_sq);
        Flags {
            is_ob,
            pos_e,
            en_ob,
            en_sq,
        }
    };
    let unmake_flags = |b: &mut Builder, fl: Flags| {
        let control = b.control();
        b.cx(fl.en_ob, fl.en_sq);
        b.cx(control, fl.en_sq);
        b.free(fl.en_sq);
        erase_and(b, control, fl.is_ob, fl.en_ob, false);
    };
    // `e`: the enables exist only around the Majorana; until then only `is_ob` is read.
    let held = match r.flags {
        Some(f) => Some(f),
        None if nw.flags => None,
        None => Some(make_flags(b)),
    };
    let is_ob = held.map_or_else(|| r.angle_q[t.slot_bits()], |f| f.is_ob);
    led.stage(b, "copy: RPREP angle read");
    // `q`: the outer q field is not read again until RPREP^dagger.
    let q_bits = r.outer_q.as_ref().map(|_| {
        r.q.iter()
            .map(|&q| {
                let bit = b.new_bit();
                let mut op = crate::circuit::Op::new(crate::circuit::OperationType::Hmr);
                op.q_target = q.0;
                op.c_target = bit.0;
                b.emit(op);
                bit
            })
            .collect::<Vec<Bit>>()
    });

    // 3. SELECT.
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
        let js = t.chunk_rotations(c);
        let base = js.start;
        let order: Vec<usize> = if reverse {
            js.rev().collect()
        } else {
            js.collect()
        };
        for j in order {
            let (pm, qm) = t.modes[j];
            for &s in spins {
                b.givens_modes(2 * pm + s, 2 * qm + s, r.angle_regs[j - base]);
            }
        }
    };
    let transition = |b: &mut Builder, from: usize, to: usize| {
        let diff = |v: u64| xor(&t.lean_word(v, from, false), &t.lean_word(v, to, false));
        ragged_read(b, t, &bsel, &r.hi, r.angle_q, &diff);
    };
    let sel = p.swap.then(|| spin_select_on(b, r.s0, r.s1, is_ob));
    if let Some(g) = sel {
        // F^dagger before, F after: F^s V M V^dagger F^-s. (F^dagger = Z_S1 F Z_S1 would hand the
        // tracker two 54-qubit Z frames per copy, past its 512-factor limit.)
        b.spin_swap_dg(g);
        if nw.flags {
            spin_unselect_on(b, r.s0, r.s1, is_ob, g);
        }
        led.stage(b, "copy: SELECT controlled spin swap F^-s");
    }
    z_s(b);
    for c in (0..t.chunks).rev() {
        if c != last {
            transition(b, c + 1, c);
            led.stage(b, "copy: RPREP angle chunk transition");
        }
        rotate(b, c, true);
    }
    z_s(b);
    led.stage(b, "copy: SELECT V^dagger");
    // The lean layout forms id = [b = B] here, from `b` (unchanged through SELECT: a square's
    // `lo` is 0; on a one-body lane `en_sq` is 0 and `id` is not read), and erases it at once.
    if nw.majorana_cut || r.pass_bit.is_some() {
        assert!(
            p.swap && t.lean && held.is_none(),
            "p and j need e, the lean layout and SpinSwap"
        );
        majorana_narrow(b, t, r, &bsel, neg, nw.majorana_cut);
        led.stage(b, "copy: SELECT Majorana and controls");
    }
    let (id, id_chain) = if nw.majorana_cut || r.pass_bit.is_some() {
        (neg, None)
    } else if t.lean {
        let (q, chain) = equals_const(b, &bsel, t.spec.b as u64);
        (q, Some(chain))
    } else {
        (main[k_i + 1], None)
    };
    let lane = Lane {
        x: r.index[0],
        s0: r.s0,
        s1: r.s1,
        pass: r.pass,
    };
    if !(nw.majorana_cut || r.pass_bit.is_some()) {
        let fl = held.unwrap_or_else(|| make_flags(b));
        if p.swap {
            majorana_spin0_on(b, lane, fl, neg, id);
        } else {
            majorana_on(b, lane, fl, neg, id);
        }
        if held.is_none() {
            unmake_flags(b, fl);
        }
        if let Some(chain) = id_chain {
            unequals_const(b, &bsel, t.spec.b as u64, chain);
        }
        led.stage(b, "copy: SELECT Majorana and controls");
    }
    for c in 0..t.chunks {
        if c != 0 {
            transition(b, c - 1, c);
            led.stage(b, "copy: RPREP angle chunk transition");
        }
        rotate(b, c, false);
    }
    led.stage(b, "copy: SELECT V");
    if let Some(made) = sel {
        let g = if nw.flags {
            spin_select_on(b, r.s0, r.s1, is_ob)
        } else {
            made
        };
        b.spin_swap(g);
        spin_unselect_on(b, r.s0, r.s1, is_ob, g);
        led.stage(b, "copy: SELECT controlled spin swap F^s");
    }

    // 4. RPREP^dagger.
    if r.flags.is_none() {
        if let Some(fl) = held {
            unmake_flags(b, fl);
        }
    }
    let fin = |v: u64| t.lean_word(t.leaf(v), last, true);
    erase_lookup_keep(b, &idx, e, r.angle_q, &fin, erase_split(e));
    if let (Some(oq), Some(m_q)) = (&r.outer_q, q_bits) {
        restore_q(b, t, r, oq, m_q, p.narrow.hot_cap);
        led.stage(b, "copy: outer q re-read");
    }
    lo_into(b, t, r, &bsel);
    led.stage(b, "copy: RPREP^dagger angle erasure");

    // 5. Inner PREPARE^dagger.
    if let Some(m_a) = alt_bits {
        inner_unprepare_dropped(
            b,
            t,
            r,
            p,
            (read, &words, index, limit),
            main,
            (keep_bits, m_a),
            cmp.lt(),
        );
        led.stage(
            b,
            "copy: inner item measured, merged fixup, gated keep re-read",
        );
        toggle_pass(b, r);
        led.stage(b, "copy: inner alias read erased");
        return;
    }
    choose_alt(b, cmp.lt(), &main, alt);
    let Some(a2) = nw.inner_keep else {
        cmp.erase_by(b, &r.draw, keep, nw.gated);
        for (&i, &q) in r.index.iter().zip(&bsel) {
            b.cx(i, q);
        }
        bsel.into_iter().for_each(|q| b.free(q));
        led.stage(b, "copy: inner alt swap and keep test undone");
        // `h` (explicit): the plain inner erasure's one-hot is capped too.
        let h = if nw.cap_erase {
            erase_split(limit).min(nw.hot_cap)
        } else {
            erase_split(limit)
        };
        qroam::erase(b, &index, read, &words, h);
        toggle_pass(b, r);
        led.stage(b, "copy: inner alias read erased");
        return;
    };
    for (&i, &q) in r.index.iter().zip(&bsel) {
        b.cx(i, q);
    }
    bsel.into_iter().for_each(|q| b.free(q));
    led.stage(b, "copy: inner alt swap undone");
    let keep_word = |x: u64| narrow::low_bits(&words(x), mu);
    let lt = cmp.lt();
    let extra = narrow::gated_lt_erase(
        b,
        lt,
        &r.draw,
        &index,
        limit,
        &keep_word,
        narrow::Reread { a: a2, fix: None },
        narrow::Fault::None,
    );
    led.stage(b, "copy: gated inner keep re-read");
    let (a1, a2) = (p.inner_a.min(k_i).min(index.len()), a2.min(index.len()));
    let w1 = t.inner_word_bits();
    let mut first = read.into_junk();
    first.extend(keep_bits.expect("measured with the comparison"));
    first.extend(measure(b, out[mu..].to_vec()));
    let full =
        narrow::read_full(index.len(), limit, a1).max(narrow::read_full(index.len(), limit, a2));
    let c1 = |x: u64| narrow::read_content(x, a1, w1, limit, &words);
    let c2 = |x: u64| narrow::read_content(x, a2, mu, limit, &keep_word);
    narrow::fixup_parts(
        b,
        &index,
        full,
        &[(&first, &c1), (&extra, &c2)],
        qroam::Split {
            h: erase_split(limit),
            hot: None,
        },
    );
    toggle_pass(b, r);
    led.stage(b, "copy: inner alias read erased");
}
