//! `sa-toff`: Low et al.'s step (`super::emit`, `inner::copy`) with Toffoli levers switched on
//! by [`Tweaks`]. The published structure, lane map, SELECT and spin swap are unchanged; what
//! changes is how the alias words are laid out, how the alt swaps and the lookups' one-hot
//! registers are erased, and (optionally) how long the comparators' carries live. README
//! section 6 lists each lever with its measured effect.
//!
//! **Measured undo of an alt swap** (`Tweaks::measured_undo`). After `choose_alt` the registers
//! hold `main' = alt ^ lt D` and `alt' = main ^ lt D` with `D = main ^ alt`. `alt' ^= main'`
//! makes `alt' = D`, a function of the lookup index only. Measuring `main'` in the X basis with
//! outcomes `m` leaves `(-1)^(m . alt) (-1)^(lt (m . D))`: the second factor is cancelled by a
//! classically conditioned `CZ(lt, D_j)` per outcome (Clifford), the first is a function of the
//! index and joins the lookup's fixup, as does `D` when `alt'` is measured with the rest of the
//! word. So the swap costs its `w` Toffolis once, not twice.
//!
//! **Compact outer data** (`Tweaks::compact_outer`). A one-body item's `q` is `ones | r`
//! (past every square, so the inner lookup reads nothing there) and `lo` is not stored: the
//! RPREP index's low half gets `r mod 2^k_i` by `b ^= is_ob AND q_low` (`k_i` Toffolis per
//! copy). That term is never undone: when `b` is measured its phase `(-1)^(m . is_ob q_low)` is
//! a `CZ(is_ob, q_j)` per outcome. On a one-body lane the lookup index is past the table, so
//! the fixup does not cover it; the `alt'` slots there hold `i ^ q_low`, cancelled the same way.
//!
//! **Measured one-hot** (`Tweaks::hot_erase`): `qroam::fixup`.
use super::inner::{choose_alt, erase_split, ragged_read, InnerRegs};
use super::itemfold;
use super::itemhot::{self, ItemHot};
use super::ledger::Ledger;
use super::narrow;
use super::onehot;
use super::qroam::{self, best_split, Split};
use super::tables::SaTables;
use super::Params;
use crate::circuit::{Bit, Builder, Qubit, Reg, SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE};
use crate::lanemap::sa_nested::SaNestedMap;
use crate::spec::sa::SaSpec;
use crate::walk::common::arith::{
    all_ones, carries, erase_all_ones, erase_carries, less_than, unless_than,
};
use crate::walk::common::arith_gated::unless_than_gated;
use crate::walk::common::lookup::hmr_keep;
use crate::walk::common::unary::erase_and;
use crate::walk::shared::lookup::{put, word, Word};

/// Bits `at..at + width` of `w` (`width <= 64`).
fn get(w: &[u64], at: usize, width: usize) -> u64 {
    (0..width).fold(0, |acc, j| {
        let bit = w[(at + j) / 64] >> ((at + j) % 64) & 1;
        acc | bit << j
    })
}

/// A comparison `lt = [draw < keep]`, with its carry ladder when it is kept.
struct Cmp {
    lt: Qubit,
    ladder: Vec<Qubit>,
}

fn compare(b: &mut Builder, draw: &[Qubit], keep: &[Qubit], hold: bool) -> Cmp {
    if !hold {
        return Cmp {
            lt: less_than(b, draw, keep),
            ladder: Vec::new(),
        };
    }
    draw.iter().for_each(|&q| b.x(q));
    let cs = carries(b, draw, keep, draw.len());
    draw.iter().for_each(|&q| b.x(q));
    Cmp {
        lt: cs[cs.len() - 1],
        ladder: cs,
    }
}

fn uncompare(b: &mut Builder, draw: &[Qubit], keep: &[Qubit], c: Cmp, gated: bool) {
    if c.ladder.is_empty() {
        if gated {
            unless_than_gated(b, draw, keep, c.lt);
        } else {
            unless_than(b, draw, keep, c.lt);
        }
        return;
    }
    draw.iter().for_each(|&q| b.x(q));
    erase_carries(b, draw, keep, &c.ladder);
    draw.iter().for_each(|&q| b.x(q));
}

/// `alt ^= main`, then `main` measured; `CZ(lt, alt_j)` where outcome `j` is 1. Returns the
/// outcomes (`main` is freed).
fn undo_measured(b: &mut Builder, lt: Qubit, main: &[Qubit], alt: &[Qubit]) -> Vec<Bit> {
    for (&m, &a) in main.iter().zip(alt) {
        b.cx(m, a);
    }
    let bits: Vec<Bit> = main.iter().map(|&q| b.hmr(q)).collect();
    for (&m, &a) in bits.iter().zip(alt) {
        b.cz_if(lt, a, m);
    }
    bits
}

/// `.u`: toggle the selected item's `hi` field using the square-item one-hot. On Li, square
/// items have `hi = floor((item - N) / C) < 16`. One-body items have `hi = R + (item >> 6)`:
/// the exceptional second row is a single AND into bit 4, then Clifford fan-out.
fn toggle_outer_hi(
    b: &mut Builder,
    hot: &ItemHot,
    item: &[Qubit],
    is_ob: Qubit,
    hi: &[Qubit],
    n: u64,
    c: u64,
    undo: bool,
) {
    assert_eq!((n, c, hi.len()), (76, 19, 5), ".u is built for Li");
    let square = |b: &mut Builder| itemhot::fan(b, hot, hi, &|v| (v - n) / c);
    let ob_base = |b: &mut Builder| {
        for &q in &hi[..4] {
            b.cx(is_ob, q);
        }
    };
    let ob_second = |b: &mut Builder| b.ccx(is_ob, item[6], hi[4]);
    let ob_fix = |b: &mut Builder| {
        for &q in &hi[..4] {
            b.cx(hi[4], q);
        }
    };
    if undo {
        ob_fix(b);
        ob_second(b);
        ob_base(b);
        square(b);
    } else {
        square(b);
        ob_base(b);
        ob_second(b);
        ob_fix(b);
    }
}

/// Lever `_`: `index ^= lt AND (chosen ^ unchosen)` with `chosen ^= unchosen` made in place
/// (CNOTs) and undone; one Toffoli per index bit. Its own inverse.
pub(super) fn index_swap(
    b: &mut Builder,
    lt: Qubit,
    chosen: &[Qubit],
    unchosen: &[Qubit],
    index: &[Qubit],
) {
    for (&c, &u) in chosen.iter().zip(unchosen) {
        b.cx(u, c);
    }
    for (&c, &i) in chosen.iter().zip(index) {
        if !super::onehot::fault_is(162) {
            b.ccx(lt, c, i);
        }
    }
    for (&c, &u) in chosen.iter().zip(unchosen) {
        b.cx(u, c);
    }
}

fn split_for(hot: bool, bits: usize, limit: u64) -> Split {
    if hot {
        best_split(bits, limit, true)
    } else {
        Split {
            h: erase_split(limit),
            hot: None,
        }
    }
}

/// Lever `E`'s data for the copy: the one-hot register allocated before the
/// nested block, and each rotation's pivots inside it (`onehot::Pivot`).
struct Embed {
    hot: Vec<Qubit>,
    piv: Vec<onehot::Pivot>,
}

/// [`super::emit`] with the levers of `p.tw`.
#[allow(clippy::too_many_lines)]
pub fn emit(spec: &SaSpec, map: &SaNestedMap, b: &mut Builder, p: Params) -> Ledger {
    let tw = p.tw;
    assert!(
        !tw.compact_outer || tw.measured_undo,
        "compact outer data needs the measured undo"
    );
    assert!(
        p.chunks <= 1 && !p.outer_erase && !p.dense,
        "the sa-toff levers are built on the published (one-chunk, padded-index) step"
    );
    // Lever `s`: deliver each leaf's angles as the network of `+u` or `-u`,
    // whichever keeps its last angle below `2^(beta - 1)` (`signnorm.rs`). Only the angles change;
    // the lane map is the caller's, built from the spec's coefficients.
    let normed;
    let spec = if tw.sign_norm {
        assert!(tw.onehot, "lever s is built for the one-hot RPREP");
        normed = super::signnorm::sign_normalized(spec);
        &normed
    } else {
        spec
    };
    let t = if tw.item_outer {
        SaTables::with_items(spec, map, tw.derive_id)
    } else {
        SaTables::with(spec, map, tw.derive_id, tw.compact_outer)
    };
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
    let slots = t.slots(1);
    // One-hot RPREP (lever `u`): one angle register, the widest rotation, and one `Givens`
    // register per rotation over its prefix; otherwise the whole angle word.
    let mut embed: Option<Embed> = None;
    let (angle_q, angle_regs): (Vec<Qubit>, Vec<Reg>) = if tw.embed {
        assert!(
            tw.onehot && tw.hot_groups == 0 && !tw.measured_unload,
            "lever E needs the unsplit one-hot (and its own unload)"
        );
        let hot = b.alloc_n(onehot::leaves(&t));
        let piv: Vec<onehot::Pivot> = (0..t.modes.len()).map(|j| onehot::pivot(&t, j)).collect();
        // A dependent angle bit (toy specs only: fewer leaves than bits) gets its own qubit.
        let nx = piv.iter().map(|pv| pv.extras().len()).max().unwrap_or(0);
        let extra = b.alloc_n(nx);
        let regs = piv
            .iter()
            .map(|pv| b.register(&onehot::pivot_register(&hot, &extra, pv)))
            .collect();
        embed = Some(Embed { hot, piv });
        (extra, regs)
    } else if tw.onehot {
        // Lever `P`: a rotation whose `pi` bit moves to the chain's edge gets a register one
        // qubit narrower.
        let pe = pi_edge_set(&t, tw.pi_edge);
        let ws: Vec<usize> = t
            .widths
            .iter()
            .zip(&pe)
            .map(|(&w, &on)| if on { w - 1 } else { w })
            .collect();
        let q = b.alloc_n(ws.iter().copied().max().unwrap_or(1));
        let regs = ws.iter().map(|&w| b.register(&q[..w])).collect();
        (q, regs)
    } else {
        let q = b.alloc_n(t.angle_bits());
        let regs = (0..t.modes.len())
            .map(|j| b.register(&q[t.at[j]..t.at[j] + t.widths[j]]))
            .collect();
        (q, regs)
    };
    let inner_reg = b.inner_register(u_o, w);

    led.start(b);
    b.segment(SEG_PREPARE);
    let mu = mu_o as usize;
    let buckets = 1u64 << k_o;
    // Word layout: `keep | own (od bits) | alt (d bits)`; the selected register `main` is
    // `copy | own` where `copy` (item layout only) is the bucket index's low `kx` bits.
    let derive_hi = tw.outer_hi_from_hot;
    assert!(
        !derive_hi || (t.item_layout && tw.item_hot && tw.item_align && tw.item_inplace),
        ".u needs the aligned in-place item one-hot"
    );
    let (d, od) = if derive_hi {
        (t.kx + 2, 2)
    } else if t.item_layout {
        (t.item_fields().end, t.h + 2)
    } else {
        (t.outer_data_bits(), t.outer_data_bits())
    };
    let words = |i: u64| {
        if derive_hi {
            t.item_word_no_hi(i)
        } else if t.item_layout {
            t.item_word(i)
        } else {
            t.outer_word(i)
        }
    };
    // Lever `X`: the exclusive multiplexer in place of the swap network.
    let read = if tw.excl_select {
        qroam::load_excl(b, &index_o, buckets, mu + od + d, &words, p.outer_a)
    } else {
        qroam::load(b, &index_o, buckets, mu + od + d, &words, p.outer_a)
    };
    let all = read.out.clone();
    let keep: Vec<Qubit> = all[..mu].to_vec();
    let own: Vec<Qubit> = all[mu..mu + od].to_vec();
    let alt: Vec<Qubit> = all[mu + od..].to_vec();
    led.stage(b, "outer: alias QROAM read");
    let icopy: Vec<Qubit> = if t.item_layout {
        let c = b.alloc_n(t.kx);
        for (&i, &q) in index_o.iter().zip(&c) {
            b.cx(i, q);
        }
        c
    } else {
        Vec::new()
    };
    let main_small: Vec<Qubit> = [&icopy[..], &own[..]].concat();
    let cmp = compare(b, &draw_o, &keep, tw.outer_ladder);
    let lt = cmp.lt;
    choose_alt(b, lt, &main_small, &alt);
    // Lever `d`: the unchosen slot, turned into `D = own ^ alt` (a function of the bucket alone)
    // by `alt ^= main`, is measured now instead of at the end; the `lt`-dependent half of the
    // chosen slot's phase is then cancelled by a fixup controlled by `lt` (`undrop` below).
    let early_alt: Option<Vec<Bit>> = tw.drop_alt.then(|| {
        for (&m, &a) in main_small.iter().zip(&alt) {
            b.cx(m, a);
        }
        alt.iter().map(|&q| b.hmr(q)).collect()
    });
    // Lever `k`: the keep register is measured now (its phase is a function of the bucket and
    // joins the final fixup); the comparison is erased at the end by the gated re-read.
    let early_keep: Option<Vec<Bit>> = tw.keep_release.then(|| {
        assert!(
            early_alt.is_some() && !tw.outer_ladder,
            "lever k needs d (and m) and the plain outer comparison"
        );
        keep.iter().map(|&q| b.hmr(q)).collect()
    });
    let f = if t.item_layout {
        t.item_fields()
    } else {
        t.fields()
    };
    let qw = if t.item_layout { t.kx } else { t.q_b };
    let hi_reg = if derive_hi {
        b.alloc_n(t.h)
    } else {
        Vec::new()
    };
    let main: Vec<Qubit> = if derive_hi {
        [&icopy[..], &hi_reg[..], &own[..]].concat()
    } else {
        main_small.clone()
    };
    let is_ob = main[f.is_ob];
    // Lever `W`: the keep test leaves now; UNPREPARE stands the witness
    // `[item = x_o]` in for it.
    let early_lt: Option<Bit> = tw.outer_witness.then(|| {
        assert!(
            t.item_layout && early_keep.is_some() && early_alt.is_some(),
            "lever W needs x, d and k"
        );
        b.hmr(lt)
    });
    // Lever `O`: the chosen `pos_e` slot is measured now; its outcome is
    // the one the final outer fixup cancels (the slot holds the same value until then). Each
    // copy reads `pos_e` from the one-hot instead.
    let early_pos: Option<Bit> = tw.pos_from_hot.then(|| {
        assert!(
            tw.majorana_cut && tw.onehot && early_alt.is_some(),
            "lever O needs y, the one-hot and d"
        );
        b.hmr(main[f.pos_e])
    });
    let control = b.control();
    // Lever `y` (the Majorana cut): no enables and no pass qubit across the step; the
    // copy never reads `en_ob`, `en_sq` or `pass` then, so they name the control as a
    // placeholder.
    let cut = tw.majorana_cut;
    if cut {
        assert!(
            p.swap && tw.derive_id,
            "lever y needs the SpinSwap SELECT and the derived id (i)"
        );
    }
    let (en_ob, en_sq, pass) = if cut {
        (control, control, control)
    } else {
        let en_ob = b.alloc();
        b.ccx(control, is_ob, en_ob);
        let en_sq = b.alloc();
        b.cx(control, en_sq);
        b.cx(en_ob, en_sq);
        (en_ob, en_sq, b.alloc())
    };
    let pass_bit = cut.then(|| narrow::zero_bit(b));
    led.stage(b, "outer: keep test, alt swap, enables");
    // Lever `H`: the outer item written once into a paired item one-hot,
    // present at each copy's start and end (the copy erases it after its inner read and writes it
    // again before its inner erasure).
    let ih: Option<ItemHot> = tw.item_hot.then(|| {
        assert!(
            t.item_layout && tw.measured_undo && !tw.inner_release && !tw.pad_share,
            "lever H needs x and m, and neither j nor p"
        );
        let h = if tw.item_align {
            let mut h = if tw.item_groups4 {
                // Lever `Y`: four groups on the item's two low bits.
                assert!(tw.item_inplace, "lever Y needs I");
                ItemHot::alloc_aligned_groups(
                    b,
                    t.spec.n as u64,
                    t.spec.r * t.spec.c,
                    &main[f.q..f.q + 2],
                )
            } else {
                ItemHot::alloc_aligned(b, t.spec.n as u64, t.spec.r * t.spec.c, main[f.q])
            };
            // Lever `I`: in-place expansion and measured collapse, rooted at `NOT is_ob`.
            if tw.item_inplace {
                h.inplace = Some(is_ob);
            }
            h
        } else {
            assert!(
                !tw.item_inplace,
                "lever I needs the aligned item one-hot (V)"
            );
            ItemHot::alloc(b, t.spec.n as u64, t.spec.r * t.spec.c)
        };
        itemhot::write(b, &h, &main[f.q..f.q + qw]);
        h
    });
    if derive_hi {
        toggle_outer_hi(
            b,
            ih.as_ref().unwrap(),
            &icopy,
            is_ob,
            &hi_reg,
            t.spec.n as u64,
            t.spec.c as u64,
            false,
        );
    }
    led.stage(b, "outer: item one-hot write");

    b.segment(SEG_SELECT);
    let regs = InnerRegs {
        index: index_i,
        draw: draw_i,
        s0,
        s1,
        lo: f
            .lo
            .map_or_else(Vec::new, |l| main[l..l + k_i as usize].to_vec()),
        hi: main[f.hi..f.hi + t.h].to_vec(),
        q: main[f.q..f.q + qw].to_vec(),
        pos_e: main[f.pos_e],
        is_ob,
        en_ob,
        en_sq,
        pass,
        angle_q: &angle_q,
        angle_regs: &angle_regs,
        slots: &slots,
    };
    b.nested_inner(inner_reg, |b| {
        copy(
            b,
            &t,
            &regs,
            p,
            pass_bit,
            embed.as_ref(),
            ih.as_ref(),
            &mut led,
        );
    });
    led.start(b);

    b.segment(SEG_UNPREPARE);
    if let Some(h) = &ih {
        if derive_hi {
            toggle_outer_hi(
                b,
                h,
                &icopy,
                is_ob,
                &hi_reg,
                t.spec.n as u64,
                t.spec.c as u64,
                true,
            );
            hi_reg.iter().for_each(|&q| b.free(q));
        }
        let lim = t.spec.outer_items() as u64;
        itemhot::erase(b, h, &regs.q, lim, split_for(tw.hot_erase, qw, lim));
    }
    if !cut {
        b.free(pass);
        b.cx(en_ob, en_sq);
        b.cx(control, en_sq);
        b.free(en_sq);
        erase_and(b, control, is_ob, en_ob, false);
    }
    let split = split_for(tw.hot_erase, k_o as usize, buckets);
    // Lever `W`: the witness `[icopy = x_o]` (equal to `lt` except on self-aliased buckets), with
    // `Z` for the early outcome; only the AND's top is kept.
    let lt = match early_lt {
        Some(m1) => {
            // Literals: `icopy_j = x_j` below `kx`, `x_j = 0` above (a padding bucket there).
            let hi_x = &index_o[icopy.len()..];
            for (&i, &q) in index_o.iter().zip(&icopy) {
                b.cx(i, q);
                b.x(q);
            }
            hi_x.iter().for_each(|&q| b.x(q));
            let lits: Vec<Qubit> = icopy.iter().chain(hi_x).copied().collect();
            let (acc, mut chain) = all_ones(b, &lits);
            chain.pop();
            erase_all_ones(b, chain);
            hi_x.iter().for_each(|&q| b.x(q));
            for (&i, &q) in index_o.iter().zip(&icopy) {
                b.x(q);
                b.cx(i, q);
            }
            if !onehot::fault_is(45) {
                b.z_if(acc, m1);
            }
            acc
        }
        None => lt,
    };
    if tw.measured_undo {
        let kc = icopy.len();
        let cmask = (1u64 << kc) - 1;
        let mb = if early_alt.is_some() {
            // main holds alt ^ lt D: its outcomes leave (-1)^(m . alt(i)) (the final fixup's
            // content, as without the lever) times (-1)^(lt (m . D(i))), cancelled here.
            let mm: Vec<Bit> = main_small
                .iter()
                .enumerate()
                .map(|(j, &q)| match early_pos {
                    Some(m) if j == icopy.len() + od - 1 => m,
                    _ => b.hmr(q),
                })
                .collect();
            let d_of = |i: u64| -> Word {
                let wd = words(i);
                let mut c = word(kc + od);
                put(&mut c, 0, kc, (i & cmask) ^ get(&wd, mu + od, kc));
                put(
                    &mut c,
                    kc,
                    od,
                    get(&wd, mu, od) ^ get(&wd, mu + od + kc, od),
                );
                c
            };
            qroam::phase_fixup_ctl(b, lt, &index_o, buckets, &mm, &d_of, erase_split(buckets));
            mm
        } else {
            undo_measured(b, lt, &main_small, &alt)
        };
        let keep_word = |i: u64| narrow::low_bits(&words(i), mu);
        let a2 = p.outer_a.min(index_o.len());
        let mut self_bits: Vec<Bit> = Vec::new();
        let reread: Vec<Bit> = if early_keep.is_some() {
            let fault = if onehot::fault_is(7) {
                narrow::Fault::NoPhase
            } else {
                narrow::Fault::None
            };
            let r = narrow::Reread { a: a2, fix: None };
            let (rr, m2) = narrow::gated_lt_erase_m(
                b, lt, &draw_o, &index_o, 0, buckets, &keep_word, r, fault,
            );
            // Lever `W`: both outcomes' self-bucket remainder `(-1)^(m self(x_o))`.
            if let Some(m1) = early_lt {
                if !onehot::fault_is(46) {
                    self_bits = vec![m1, m2];
                }
            }
            rr
        } else {
            uncompare(b, &draw_o, &keep, cmp, tw.gated);
            Vec::new()
        };
        led.stage(b, "outer^-1: alt swap (measured), keep test, enables");
        let (mcopy, mown) = mb.split_at(icopy.len());
        let mk: Vec<Bit> = early_keep.unwrap_or_else(|| keep.iter().map(|&q| b.hmr(q)).collect());
        let ma: Vec<Bit> = early_alt.unwrap_or_else(|| alt.iter().map(|&q| b.hmr(q)).collect());
        let out_bits: Vec<Bit> = mk
            .into_iter()
            .chain(mown.iter().copied())
            .chain(ma)
            .collect();
        // What each slot held when measured: main' slots the alt's value, alt' slots
        // `own ^ alt` (the own item's index is the bucket's own, `i`).
        let content = |i: u64| -> Word {
            let wd = words(i);
            let own_v = get(&wd, mu, od);
            let alt_i = get(&wd, mu + od, kc);
            let alt_v = get(&wd, mu + od + kc, od);
            let mut c = word(mu + od + d);
            put(&mut c, 0, mu, get(&wd, 0, mu));
            put(&mut c, mu, od, alt_v);
            put(&mut c, mu + od, kc, (i & cmask) ^ alt_i);
            put(&mut c, mu + od + kc, od, own_v ^ alt_v);
            c
        };
        // The extra outcomes: the item copy's, then (lever `k`) the gated re-read's, 0 on the
        // lanes that did not run it (`narrow::read_content`'s layout: junk blocks, then output).
        let nr = reread.len();
        let copy_data = |i: u64| -> Word {
            let mut c = word((kc + nr).max(1));
            put(&mut c, 0, kc, get(&words(i), mu + od, kc));
            if nr > 0 && !onehot::fault_is(8) {
                let rc = narrow::read_content(i, a2, mu, buckets, &keep_word);
                for j in 0..nr {
                    put(&mut c, kc + j, 1, get(&rc, j, 1));
                }
            }
            c
        };
        // Lever `W`'s two bits after the re-read's: `self(x) = [alt item = x]`.
        let ns = self_bits.len();
        let copy_data = |i: u64| -> Word {
            let mut c = copy_data(i);
            if ns > 0 {
                let alt_i = get(&words(i), mu + od, kc);
                let is_self = u64::from(i >> kc == 0 && alt_i == i);
                let mut d = word(kc + nr + ns);
                for j in 0..kc + nr {
                    put(&mut d, j, 1, get(&c, j, 1));
                }
                for j in 0..ns {
                    put(&mut d, kc + nr + j, 1, is_self);
                }
                c = d;
            }
            c
        };
        let mut extra = mcopy.to_vec();
        extra.extend(reread);
        extra.extend(self_bits);
        qroam::erase_parts(
            b, &index_o, read, &words, out_bits, &content, extra, &copy_data, split,
        );
    } else {
        assert!(early_alt.is_none(), "lever d needs the measured undo (m)");
        choose_alt(b, lt, &main_small, &alt);
        uncompare(b, &draw_o, &keep, cmp, tw.gated);
        led.stage(b, "outer^-1: alt swap, keep test, enables");
        qroam::erase_split_by(b, &index_o, read, &words, split);
    }
    led.stage(b, "outer^-1: alias read erased");
    angle_q.into_iter().for_each(|q| b.free(q));
    if let Some(e) = embed {
        e.hot.into_iter().for_each(|q| b.free(q));
    }
    led
}

/// [`super::inner::copy`] with the levers of `p.tw`.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
/// Lever `e`: reloads `hi | is_ob` (X-measured before the inner read with outcomes
/// `m`) from the chosen item `q` by a small paired lookup over the item index
/// (`paired::load_paired`, `2^5` slots, `t.item_flags`), swaps the values into their qubits and
/// cancels the measurement's phase `(-1)^(m . v)` with `Z` on each reloaded bit whose outcome
/// was 1 (the reloaded value is the measured one).
fn reload_fields(
    b: &mut Builder,
    t: &SaTables<'_>,
    r: &InnerRegs<'_>,
    m: &[Bit],
    t_collapse: bool,
) {
    let w = t.h + 1;
    let n_items = t.spec.outer_items() as u64;
    let mask = (1u64 << w) - 1;
    let data = |x: u64| -> Word {
        let mut v = word(w);
        if x < n_items {
            v[0] = t.item_flags(x as usize) & mask;
        }
        v
    };
    let s = 5.min(r.q.len() - 1);
    let opt = super::paired::Opts {
        host: false,
        collapse: t_collapse,
        prune: false,
    };
    let rd = super::paired::load_paired(b, &r.q, 0, n_items, w, &data, s, opt);
    let targets: Vec<Qubit> =
        r.hi.iter()
            .copied()
            .chain(std::iter::once(r.is_ob))
            .collect();
    for ((&o, &q), &mk) in rd.out.iter().zip(&targets).zip(m) {
        if !onehot::fault_is(48) {
            b.swap(o, q);
        }
        b.free(o);
        if !onehot::fault_is(47) {
            b.z_if(q, mk);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn copy(
    b: &mut Builder,
    t: &SaTables<'_>,
    r: &InnerRegs<'_>,
    p: Params,
    pass_bit: Option<Bit>,
    emb: Option<&Embed>,
    ih: Option<&ItemHot>,
    led: &mut Ledger,
) {
    let tw = p.tw;
    let k_i = t.k_i;
    let mu = r.draw.len();
    // Lever `.e`: the item one-hot erasure's phase pass with sibling leaves paired.
    let phase_fn = if tw.erase_pairs {
        itemhot::phase_rooted_paired
    } else {
        itemhot::phase_rooted
    };
    let fb = t.flag_bits();
    led.start(b);
    // 1. Inner PREPARE.
    let index: Vec<Qubit> = [&r.index[..], &r.q[..]].concat();
    let (start, limit) = if t.item_layout {
        t.item_range()
    } else {
        (0, t.inner_limit())
    };
    let words = |x: u64| {
        if t.item_layout {
            t.item_inner_word(x)
        } else {
            t.inner_word(x)
        }
    };
    // Lever `p` (range.rs) swaps in the padded read; otherwise the same calls as before.
    let plan = super::pad_plan_of(t.spec, p, &t.map.inner);
    let item_word = |item: u64, i: u64| words(item << k_i | i);
    // Lever `t`: the read stores `keep | delta | alt b` with `delta = s_own ^ s_alt`
    // (0 where the keep is 0 or past the items, where `lt` is always 0), and applies
    // `(-1)^(s_alt)` in the same pass; the iteration is rooted at the control.
    let sp = tw.sign_pass;
    if tw.pad_offset {
        assert!(
            sp && ih.is_some_and(|h| h.shift <= 1),
            "lever + needs t and the two-group item one-hot (H)"
        );
    }
    if sp {
        assert!(
            (ih.is_some() || (tw.paired_lookup && tw.paired_erase))
                && fb == 1
                && tw.majorana_cut
                && tw.measured_undo
                && !tw.inner_release
                && !tw.sign_phase,
            "lever t needs H (or G with F), i, y and m, and neither j nor S"
        );
    }
    let spec_b = t.spec.b as u64;
    let imask = (1u64 << k_i) - 1;
    let tword = |x: u64| -> Word {
        let wd = words(x);
        let keep = get(&wd, 0, mu);
        let own_f = get(&wd, mu, 1);
        let alt_b = get(&wd, mu + 1, k_i);
        let alt_f = get(&wd, mu + 1 + k_i, 1);
        let delta = if keep == 0 || x & imask > spec_b || onehot::fault_is(93) {
            0
        } else {
            own_f ^ alt_f
        };
        let mut c = word(mu + 1 + k_i);
        put(&mut c, 0, mu, keep);
        put(&mut c, mu, 1, delta);
        put(&mut c, mu + 1, k_i, alt_b);
        c
    };
    let titem = |item: u64, i: u64| tword(item << k_i | i);
    let s_alt = |item: u64, i: u64| get(&words(item << k_i | i), mu + 1 + k_i, 1) == 1;
    let wbits = if sp {
        mu + 1 + k_i
    } else {
        t.inner_word_bits()
    };
    let ctl = b.control();
    // Lever `K` (with `H`): during the inner read the outer item fields of a
    // square lane are functions of the item one-hot, so the item index, `hi` and `is_ob` are
    // XORed to zero from it (one-body lanes keep their low item bits: their high item bits are 0,
    // `hi = R + (r >> k_i)` is cleared by `is_ob` and item bit `k_i`, and `is_ob` by the
    // one-hot's parity), and rebuilt in reverse after the read.
    let s_n = t.spec.n as u64;
    let ob_bits = (64 - (s_n - 1).leading_zeros()) as usize;
    let rows_r = t.spec.r as u64;
    let hi_fix = rows_r ^ (rows_r + 1);
    // Lever `D` (with `H`, `K`): during the read the one-body lanes' low item bits
    // (and `pos_e`) are the only outer data left live, and the read's output register is 0 on
    // exactly those lanes (their item one-hot is empty). So they are moved into it (CNOTs both
    // ways: on a square lane the bits are already 0) and come back after the read (a Toffoli with
    // `is_ob` per bit, then a CNOT).
    let q_from = usize::from(tw.item_align) + usize::from(tw.item_groups4);
    let stash_src: Vec<Qubit> = if tw.stash {
        r.q[q_from..ob_bits]
            .iter()
            .copied()
            .chain(std::iter::once(r.pos_e))
            .collect()
    } else {
        Vec::new()
    };
    let stash_out: Vec<Qubit> = if tw.stash && ih.is_some() {
        b.alloc_n(wbits)
    } else {
        Vec::new()
    };
    let stash_at = |bit: usize| -> Option<Qubit> {
        (!stash_out.is_empty() && bit >= q_from && bit < ob_bits).then(|| stash_out[bit - q_from])
    };
    let unstash = |b: &mut Builder| {
        // A swap controlled by `is_ob` (one Toffoli): on a one-body lane `q` is 0 and takes the
        // stash; on a square lane nothing moves, whatever `q` holds by then.
        for (&q, &o) in stash_src.iter().zip(&stash_out) {
            b.cx(q, o);
            if !onehot::fault_is(46) {
                b.ccx(r.is_ob, o, q);
            }
            b.cx(q, o);
        }
    };
    let clear_items = |b: &mut Builder, h: &ItemHot, undo: bool| {
        assert!((s_n - 1) >> k_i <= 1, "lever K: one-body rows R and R + 1");
        let two_rows = (s_n - 1) >> k_i == 1;
        let ops = |b: &mut Builder, step: usize| match step {
            // (Lever `V`: the low item bit is the aligned one-hot's group bit; it stays.)
            0 if h.aligned => itemhot::fan(b, h, &r.q[h.shift..], &|item| item >> h.shift),
            0 => itemhot::fan(b, h, &r.q, &|item| item),
            1 => {
                itemhot::fan(b, h, &r.hi, &|item| (item - s_n) / t.spec.c as u64);
                for (j, &q) in r.hi.iter().enumerate() {
                    if rows_r >> j & 1 == 1 {
                        b.cx(r.is_ob, q);
                    }
                    if two_rows && hi_fix >> j & 1 == 1 {
                        // (Lever `D`, undo: item bit `k_i` is still stashed in the output
                        // register on the one-body lanes; read it there under `is_ob`.)
                        match stash_at(k_i) {
                            Some(o) if undo => b.ccx(r.is_ob, o, q),
                            _ => b.cx(r.q[k_i], q),
                        }
                    }
                }
            }
            _ => {
                if !onehot::fault_is(34) || undo {
                    h.q.iter().for_each(|&x| b.cx(x, r.is_ob));
                }
                b.x(r.is_ob);
            }
        };
        if undo {
            // Lever `I`: the item index is rebuilt by the measured collapse itself, level by
            // level, after `hi` and `is_ob` (`itemhot::collapse` with `restore`). Lever `D`: the
            // one-body lanes' stashed bits come back once `is_ob` is (and before `hi`, which
            // reads item bit `k_i` on those lanes).
            ops(b, 2);
            ops(b, 1);
            if h.inplace.is_none() {
                unstash(b);
                ops(b, 0);
            }
        } else {
            (0..3).for_each(|st| ops(b, st));
            for &q in r.q[ob_bits..]
                .iter()
                .chain(&r.hi)
                .chain(std::iter::once(&r.is_ob))
            {
                onehot::reset_keep(b, q);
            }
        }
    };
    if let (true, Some(h)) = (tw.item_clear, ih) {
        clear_items(b, h, false);
    }
    if !stash_out.is_empty() {
        assert!(tw.item_clear, "lever D needs K");
        assert!(
            stash_src.len() <= stash_out.len(),
            "lever D: the word holds the stash"
        );
        for (&q, &o) in stash_src.iter().zip(&stash_out) {
            b.cx(q, o);
            b.cx(o, q);
            onehot::reset_keep(b, q);
        }
    }
    let (read, out) = if let (true, Some(h)) = (sp, ih) {
        let o = if stash_out.is_empty() {
            b.alloc_n(wbits)
        } else {
            stash_out.clone()
        };
        let root = (!onehot::fault_is(94)).then_some(ctl);
        if tw.pad_offset {
            // Lever `+`: the padding word `A(item)` (and its in-pass sign) is a
            // function of the item; read `word ^ A` over the rows that are not then zero, and add
            // `A` back under the control from the one-hot.
            let top = (1u64 << k_i) - 1;
            let a_of = |item: u64| titem(item, top);
            let p_of = |item: u64| s_alt(item, top);
            let tdat = |item: u64, i: u64| -> Word {
                let mut w = titem(item, i);
                let a = a_of(item);
                if !onehot::fault_is(104) {
                    w.iter_mut().zip(&a).for_each(|(x, y)| *x ^= y);
                }
                w
            };
            let pdat = |item: u64, i: u64| s_alt(item, i) ^ p_of(item);
            let pword = |item: u64, i: u64| -> Word {
                let mut w = word(1);
                w[0] = u64::from(pdat(item, i));
                w
            };
            let n_rows = itemhot::rows_needed(h, 1 << k_i, &[&tdat, &pword]);
            if tw.item_fold {
                // Lever `-`: the folded, `i_0`-refined item one-hot.
                itemfold::read_into_fold(b, h, &r.index, n_rows, &o, &tdat, root, Some(&pdat));
            } else {
                itemhot::read_into_ext(b, h, &r.index, n_rows, &o, &tdat, root, Some(&pdat));
            }
            // The in-pass sign is off by `(-1)^(P(item))` in every copy: a phase of the outer
            // item alone, which both copies apply identically, so it cancels over the step (no
            // correction needed; `fan_rooted` could apply it with Cliffords).
            itemhot::fan_rooted(b, h, ctl, &o, &a_of, None, &[]);
        } else if tw.item_fold {
            itemfold::read_into_fold(b, h, &r.index, 1 << k_i, &o, &titem, root, Some(&s_alt));
        } else {
            itemhot::read_into_ext(b, h, &r.index, 1 << k_i, &o, &titem, root, Some(&s_alt));
        }
        (None, o)
    } else if let Some(h) = ih {
        let o = if tw.item_fold {
            let o = if stash_out.is_empty() {
                b.alloc_n(t.inner_word_bits())
            } else {
                stash_out.clone()
            };
            itemfold::read_into_fold(b, h, &r.index, 1 << k_i, &o, &item_word, None, None);
            o
        } else if stash_out.is_empty() {
            itemhot::read(b, h, &r.index, 1 << k_i, t.inner_word_bits(), &item_word)
        } else {
            itemhot::read_into(b, h, &r.index, 1 << k_i, &stash_out, &item_word);
            stash_out.clone()
        };
        (None, o)
    } else if tw.paired_lookup {
        // Lever `G`: the paired unary lookup, `inner_a` low index bits
        // one-hot.
        assert!(
            plan.is_none() && !tw.inner_release,
            "lever P replaces the QROAM read (not with p or j)"
        );
        // Lever `e`: `hi` and `is_ob` are functions of the chosen item `q`, so they
        // leave by X measurement before the read and are reloaded after it (`reload_fields`).
        let released: Option<Vec<Bit>> = tw.field_release.then(|| {
            r.hi.iter()
                .chain(std::iter::once(&r.is_ob))
                .map(|&q| hmr_keep(b, q))
                .collect()
        });
        let popt = super::paired::Opts {
            host: tw.slot_host,
            collapse: tw.slot_collapse,
            prune: tw.pad_runs,
        };
        let rd = if sp {
            // Lever `t` on the paired lookup: the word is
            // `keep | delta | alt b`, the group iteration is rooted at the control and the alt
            // sign is applied in the same pass (Cliffords).
            let s_alt_x = |x: u64| get(&words(x), mu + 1 + k_i, 1) == 1;
            let root = (!onehot::fault_is(94)).then_some(ctl);
            let load = if popt.prune {
                super::paired::load_paired_pruned
            } else {
                super::paired::load_paired_ext
            };
            super::range::InnerRead::Qroam(load(
                b,
                &index,
                start,
                limit,
                wbits,
                &tword,
                p.inner_a,
                popt,
                root,
                Some(&s_alt_x),
            ))
        } else {
            let load = if popt.prune {
                super::paired::load_paired_pruned
            } else {
                super::paired::load_paired_ext
            };
            super::range::InnerRead::Qroam(load(
                b,
                &index,
                start,
                limit,
                t.inner_word_bits(),
                &words,
                p.inner_a,
                popt,
                None,
                None,
            ))
        };
        if let Some(m) = released {
            reload_fields(b, t, r, &m, tw.slot_collapse);
        }
        let o = rd.out().to_vec();
        (Some(rd), o)
    } else {
        let rd = super::range::inner_load(
            b,
            &index,
            start,
            limit,
            t.inner_word_bits(),
            &words,
            p.inner_a.min(k_i),
            plan,
        );
        let o = rd.out().to_vec();
        (Some(rd), o)
    };
    led.stage(b, "copy: inner alias QROAM read");
    let ih_lim = t.spec.outer_items() as u64;
    let ih_split = split_for(tw.hot_erase, r.q.len(), ih_lim);
    if let Some(h) = ih {
        if tw.item_clear {
            clear_items(b, h, true);
        }
        match h.inplace {
            Some(ob) => {
                itemhot::collapse(b, h, &r.q, ob, tw.item_clear);
                // Lever `D` with `I`: the stash comes back once the one-hot is gone.
                if tw.item_clear {
                    unstash(b);
                }
            }
            None => itemhot::erase(b, h, &r.q, ih_lim, ih_split),
        }
    }
    let keep = out[..mu].to_vec();
    let own_flags = out[mu..mu + fb].to_vec();
    let alt = out[mu + fb..].to_vec();
    let bsel = b.alloc_n(k_i);
    for (&i, &q) in r.index.iter().zip(&bsel) {
        b.cx(i, q);
    }
    let cmp = compare(b, &r.draw, &keep, tw.keep_ladder);
    let lt = cmp.lt;
    // Lever `S`: the square sign is a lane phase, applied now as
    // `(-1)^(t s_alt) (-1)^(t lt (s_own ^ s_alt))` with `t = control AND NOT is_ob` (the
    // Majorana's `en_sq`): `s_chosen = lt ? s_own : s_alt`. Both sign slots are then X-measured
    // (contents `s_own(x)`, `s_alt(x)`: functions of the lookup index, cancelled by its fixup).
    let early_signs: Option<(Bit, Bit)> = tw.sign_phase.then(|| {
        assert!(
            fb == 1 && tw.majorana_cut && tw.measured_undo && !tw.inner_release,
            "lever S needs i (one flag bit), y and m, and not j"
        );
        let (s_own, s_alt) = (own_flags[0], alt[k_i]);
        let control = b.control();
        let tq = b.alloc();
        b.x(r.is_ob);
        b.ccx(control, r.is_ob, tq);
        b.x(r.is_ob);
        if !onehot::fault_is(40) {
            b.cz(tq, s_alt);
        }
        b.cx(s_alt, s_own);
        let u = b.alloc();
        b.ccx(tq, lt, u);
        if !onehot::fault_is(41) {
            b.cz(u, s_own);
        }
        erase_and(b, tq, lt, u, false);
        b.cx(s_alt, s_own);
        erase_and(b, control, r.is_ob, tq, true);
        (b.hmr(s_own), b.hmr(s_alt))
    });
    // Lever `t`: `(-1)^(lt delta)` (delta is 0 off the control-1 square lanes), then `delta`
    // leaves; its content joins the erasure's fixup.
    let early_delta: Option<Bit> = sp.then(|| {
        let d = own_flags[0];
        if !onehot::fault_is(92) {
            b.cz(lt, d);
        }
        b.hmr(d)
    });
    let (main, mut alt): (Vec<Qubit>, Vec<Qubit>) = if early_signs.is_some() || sp {
        (bsel.clone(), alt[..k_i].to_vec())
    } else {
        ([&bsel[..], &own_flags[..]].concat(), alt)
    };
    choose_alt(b, lt, &main, &alt);
    // Lever `_`: the unchosen index moves into the inner index register for the
    // SELECT window. `i = lt ? chosen : unchosen`, so `i ^= lt (chosen ^ unchosen)` leaves the
    // unchosen index there (k_i Toffolis), and the alt register, now equal to it, is cleared by
    // CNOTs and freed. Undone at PREPARE^dagger, where `lt` is back (`index_unhost`).
    if tw.index_host {
        assert!(
            alt.len() == k_i && tw.measured_undo && !tw.inner_release && stash_out.is_empty(),
            "lever _ needs the k_i-bit alt slot (t or S), m, and not j or D"
        );
        index_swap(b, lt, &bsel, &alt, &r.index);
        for (&a, &i) in alt.iter().zip(&r.index) {
            if !onehot::fault_is(160) {
                b.cx(i, a);
            }
        }
        alt.iter().for_each(|&a| b.free(a));
        alt.clear();
    }
    // Lever `R`: `lt` leaves now and is recomputed from the held keep at
    // PREPARE^dagger (`Z` there cancels this outcome's phase).
    let early_lt: Option<Bit> = tw.lt_release.then(|| {
        assert!(
            !tw.keep_ladder && tw.measured_undo && !tw.inner_release,
            "lever R needs m and the plain inner comparison, and not j"
        );
        b.hmr(lt)
    });
    // Lever `j`: the inner keep register is measured now (its phase joins the read's fixup);
    // the comparison is erased at PREPARE^dagger by the gated re-read of `keep` alone.
    let early_keep: Option<Vec<Bit>> = tw.inner_release.then(|| {
        assert!(
            tw.measured_undo && !tw.keep_ladder && !tw.pad_share,
            "lever j needs m, the plain inner comparison and the unpadded read"
        );
        keep.iter().map(|&q| b.hmr(q)).collect()
    });
    // Lever `~`: the inner keep register is measured now, as with `j`, but `lt` is
    // erased at PREPARE^dagger by the outcome-gated re-read from the item one-hot
    // (`itemhot::gated_lt_erase_hot`) instead of the QROAM range read.
    let hot_keep: Option<Vec<Bit>> = tw.hot_keep_release.then(|| {
        assert!(
            ih.is_some()
                && tw.measured_undo
                && tw.index_host
                && (early_signs.is_some() || sp)
                && !tw.lt_release
                && !tw.keep_ladder
                && !tw.pad_share
                && stash_out.is_empty()
                && !tw.inner_release,
            "lever ~ needs H, m, _ and S or t, and none of R, l, p, D, j"
        );
        let mk: Vec<Bit> = keep.iter().map(|&q| b.hmr(q)).collect();
        if onehot::fault_is(205) {
            // (Mutant 205: the outcomes left out of the erasure pass.)
            return (0..mk.len()).map(|_| narrow::zero_bit(b)).collect();
        }
        mk
    });
    let neg: Option<Qubit> = (early_signs.is_none() && !sp).then(|| main[k_i]);
    led.stage(b, "copy: inner keep test and alt swap");

    // 2. RPREP.
    if r.lo.is_empty() {
        for (j, &q) in bsel.iter().enumerate() {
            b.ccx(r.is_ob, r.q[j], q);
        }
    } else {
        for (&o, &q) in r.lo.iter().zip(&bsel) {
            b.cx(o, q);
        }
    }
    let idx: Vec<Qubit> = [&bsel[..], &r.hi[..]].concat();
    let e = t.net_limit();
    let angles = |v: u64| t.angle_word(v);
    // Lever `J` (with `b`): `id' = [b = B] AND NOT is_ob` is made here, before
    // the one-hot exists, and erased after RPREP^dagger, so its AND chains are never live
    // together with the one-hot (they were the SELECT peak's last few qubits).
    let spec_b0 = t.spec.b as u64;
    let lits_pre: Vec<Qubit> = [&bsel[..], &[r.is_ob][..]].concat();
    let flip_pre = |b: &mut Builder| {
        for (j, &q) in bsel.iter().enumerate() {
            if spec_b0 >> j & 1 == 0 {
                b.x(q);
            }
        }
        b.x(r.is_ob);
    };
    let pre_id: Option<Qubit> = (tw.early_id && tw.checkpoint).then(|| {
        flip_pre(b);
        let (acc, mut chain) = all_ones(b, &lits_pre);
        chain.pop();
        erase_all_ones(b, chain);
        flip_pre(b);
        acc
    });
    let write_from = b.ops().len();
    let hot = if tw.onehot {
        let mut h = if tw.class_inplace {
            // The unsplit one-hot (`G = 1`) is the single-group case of the class
            // layout (every class one virtual row): written in place, erased by the measured
            // collapse with no unfold (no group bits).
            assert!(
                tw.in_range && !tw.fold && (emb.is_none() || tw.hot_groups == 0),
                "lever C needs r, and replaces f (with E only unsplit)"
            );
            onehot::write_classes_pre(
                b,
                t,
                &bsel,
                &r.hi,
                tw.hot_groups.max(1),
                tw.class_pack,
                tw.class_mixed,
                emb.map(|e| &e.hot[..]),
                tw.graft,
            )
        } else if tw.fold {
            assert!(
                tw.in_range && tw.hot_groups > 1,
                "lever f needs the in-range iteration (r) and a split one-hot"
            );
            onehot::write_folded_early(b, t, &bsel, &r.hi, tw.hot_groups, tw.early_flags)
        } else {
            onehot::write_into(
                b,
                t,
                &bsel,
                &r.hi,
                tw.in_range,
                tw.hot_groups,
                emb.map(|e| &e.hot[..]),
            )
        };
        h.paired = tw.paired_groups;
        h.shared = tw.shared_triple;
        h.sandwich = tw.triple_sandwich;
        Some(h)
    } else {
        ragged_read(b, t, &bsel, &r.hi, r.angle_q, &angles);
        None
    };
    led.stage(b, "copy: RPREP angle read");

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
    // With the one-hot, `cur` is the rotation whose angle the register holds; each Givens is
    // preceded by the fan-out transition to its own angle. The register is unloaded and reset
    // around the Majorana (so it is not live there: the id chain and the pivot's AND are) and
    // after `V`.
    let mut cur: Option<usize> = None;
    // Lever `P`: the register holds an edge rotation's angle without its `pi` bit.
    let pe = pi_edge_set(t, tw.pi_edge);
    if pe.iter().any(|&x| x) {
        assert!(
            hot.is_some() && emb.is_none(),
            "lever P needs the one-hot (not E)"
        );
    }
    let pi_top = usize::from(t.spec.beta) - 1;
    let ang = |e: usize, j: usize| {
        let a = onehot::angle(t, e, j);
        if pe[j] {
            a & ((1u64 << pi_top) - 1)
        } else {
            a
        }
    };
    let pi_phase = |b: &mut Builder| {
        let Some(h) = &hot else { return };
        let nl = onehot::leaves(t);
        for (j, _) in pe.iter().enumerate().filter(|(_, &x)| x) {
            let top = |e: usize, _j: usize| onehot::angle(t, e, j) >> pi_top & 1;
            let q = b.alloc();
            onehot::transition_by(b, h, &[q], nl, &top, None, Some(0));
            let (pm, qm) = t.modes[j];
            for &s in spins {
                for m in [pm, qm] {
                    // (Mutant 47 drops the phase; 48 drops it on the pair's second mode.)
                    if onehot::fault_is(47) || (onehot::fault_is(48) && m == qm) {
                        continue;
                    }
                    let sq = b.system(2 * m + s);
                    b.cz(q, sq);
                }
            }
            onehot::unload_measured(b, h, &[q], nl, &top, 0);
            b.free(q);
        }
    };
    // Lever `q<b>`: rank-scheduled delivery replaces the transitions.
    let deliver = (tw.rank_hold > 0).then(|| {
        let h = hot.as_ref().expect("lever q needs the one-hot");
        assert!(
            emb.is_none() && !pe.iter().any(|&x| x),
            "lever q replaces E and P"
        );
        // The states the RPREP write can leave (care states of the delivered functions).
        let inputs: Vec<Qubit> = bsel.iter().chain(r.hi.iter()).copied().collect();
        // A lane carries a leaf's index, or a square row's identity item `lo = B`.
        let (kb, sp) = (bsel.len(), t.spec);
        let valid = |a: u64| {
            let (lo, hi) = (a & ((1 << kb) - 1), a >> kb);
            onehot::leaf_of(t, hi << t.k_i | lo).is_some()
                || (hi < sp.r as u64 && lo == sp.b as u64)
        };
        let reach = super::rankdel::reachable(&b.ops()[write_from..], &inputs, &valid, h);
        std::cell::RefCell::new(super::rankdel::Deliver::new(
            h,
            onehot::leaves(t),
            r.angle_q,
            &ang,
            t.modes.len(),
            tw.rank_hold as usize,
            tw.rank_mode,
            reach.as_deref(),
            tw.rank_park as usize,
        ))
    });
    let rotate = |b: &mut Builder, reverse: bool, cur: &mut Option<usize>| {
        let order: Vec<usize> = if reverse {
            (0..t.modes.len()).rev().collect()
        } else {
            (0..t.modes.len()).collect()
        };
        for j in order {
            if let (Some(h), Some(d)) = (&hot, &deliver) {
                d.borrow_mut().step(b, h, j);
                *cur = Some(j);
            } else if let Some(h) = &hot {
                // (Mutant 4 skips the first transition of V: rotation 0 is already loaded, so
                // it skips the move to rotation 1.)
                if !(onehot::fault_is(4) && !reverse && j == 1) {
                    if let Some(e) = emb {
                        // Lever `E`: unload the previous rotation's pivots, load this one's.
                        if *cur != Some(j) {
                            if let Some(c) = *cur {
                                onehot::embed(b, t, h, r.angle_q, &e.piv[c], c, false);
                            }
                            onehot::embed(b, t, h, r.angle_q, &e.piv[j], j, true);
                        }
                    } else {
                        let nl = onehot::leaves(t);
                        onehot::transition_by(b, h, r.angle_q, nl, &ang, *cur, Some(j));
                        // Lever `P`: register bits above this rotation's width are 0 on every
                        // lane once the pass leaves a wider rotation; reset them (kept
                        // allocated) so they are not live until a wider rotation needs them.
                        if tw.hold_maj {
                            if let Some(c) = *cur {
                                let weff = |x: usize| t.widths[x] - usize::from(pe[x]);
                                let (wc, mut wj) = (weff(c), weff(j));
                                // Mutant 60: one bit too many (bit `w_j - 1` is live).
                                if onehot::fault_is(60) && wc > wj {
                                    wj -= 1;
                                }
                                for &q in r.angle_q.get(wj..wc).unwrap_or(&[]) {
                                    onehot::reset_keep(b, q);
                                }
                            }
                        }
                    }
                    *cur = Some(j);
                }
            }
            let (pm, qm) = t.modes[j];
            for &s in spins {
                b.givens_modes(2 * pm + s, 2 * qm + s, r.angle_regs[j]);
            }
        }
    };
    // Lever `b`: index checkpoint. `b`, `hi` and `is_ob` are functions of
    // the leaf the one-hot holds (`onehot::leaf_fields`), so during `V^dagger` and `V` they are
    // XORed to zero from the one-hot (`onehot::transition_by` with the fields as a one-rotation
    // table: CNOTs, plus with a split one-hot one Toffoli per group pair and bit) and rebuilt the
    // same way. `is_ob` alone is rebuilt around the Majorana. The Majorana's
    // `id' = [b = B] AND NOT is_ob` is made before the clear and held instead of `id` (a square
    // identity lane reaches the in-range leaf `lo'`, so its `b` clears to `B ^ lo'`, fixed by
    // `id'`; on one-body lanes `id` is never read, so `id'` = 0 there is equivalent).
    let ck = tw.checkpoint;
    let spec_b = t.spec.b as u64;
    let h_bits = r.hi.len();
    let ck_targets: Vec<Qubit> = [&bsel[..], &r.hi[..], &[r.is_ob][..]].concat();
    // Lever `.g`: a grafted leaf's `b` holds its moved `lo` while the one-hot is live.
    let gmask = onehot::graft_masks(
        t,
        usize::from(tw.hot_groups.max(1)),
        tw.class_mixed,
        tw.graft && tw.class_inplace,
    );
    let ck_value = |e: usize, _j: usize| -> u64 {
        let (row, lo, ob) = onehot::leaf_fields(t, e);
        let lo = if onehot::fault_is(123) {
            lo
        } else {
            lo ^ gmask[e]
        };
        lo | row << k_i | ob << (k_i + h_bits)
    };
    let ob_value = |e: usize, _j: usize| -> u64 { onehot::leaf_fields(t, e).2 };
    let id_fix = spec_b ^ super::tables::leaf_in_range(spec_b, k_i, spec_b);
    let lits7: Vec<Qubit> = [&bsel[..], &[r.is_ob][..]].concat();
    let flip7 = |b: &mut Builder| {
        for (j, &q) in bsel.iter().enumerate() {
            if spec_b >> j & 1 == 0 {
                b.x(q);
            }
        }
        b.x(r.is_ob);
    };
    let n_leaves = onehot::leaves(t);
    let ck_fan = |b: &mut Builder, h: &onehot::Hot, idp: Qubit| {
        onehot::transition_by(b, h, &ck_targets, n_leaves, &ck_value, None, Some(0));
        if !onehot::fault_is(26) {
            for (j, &q) in bsel.iter().enumerate() {
                if id_fix >> j & 1 == 1 {
                    b.cx(idp, q);
                }
            }
        }
    };
    let held_id: Option<Qubit> = if let Some(p) = pre_id {
        Some(p)
    } else if ck {
        assert!(
            tw.majorana_cut && tw.derive_id && tw.in_range && p.swap && lits7.len() > 2,
            "lever b needs y, i, r, the SpinSwap SELECT and k_i >= 2"
        );
        flip7(b);
        let (acc, mut chain) = all_ones(b, &lits7);
        chain.pop();
        erase_all_ones(b, chain);
        flip7(b);
        Some(acc)
    } else {
        None
    };
    let mut sel = p.swap.then(|| super::inner::spin_select(b, r));
    if let Some(g) = sel {
        b.spin_swap_dg(g);
        // Lever `N`: the spin select leaves until `F^s` (an AND, erased by
        // measurement while `is_ob` is still held).
        if tw.spin_unload {
            super::inner::spin_unselect(b, r, g);
            sel = None;
        }
        led.stage(b, "copy: SELECT controlled spin swap F^-s");
    }
    if let (Some(idp), Some(h)) = (held_id, &hot) {
        if tw.measured_ck {
            // Lever `B`: the identity fix first, so the targets hold the
            // leaf fields exactly, then an X measurement with a Clifford fixup on the one-hot.
            if !onehot::fault_is(42) {
                for (j, &q) in bsel.iter().enumerate() {
                    if id_fix >> j & 1 == 1 {
                        b.cx(idp, q);
                    }
                }
            }
            onehot::unload_measured(b, h, &ck_targets, n_leaves, &ck_value, 0);
        } else {
            if !onehot::fault_is(25) {
                ck_fan(b, h, idp);
            }
            for &q in &ck_targets {
                onehot::reset_keep(b, q);
            }
        }
    }
    let unload = |b: &mut Builder, cur: &mut Option<usize>, fault: bool| {
        if let (Some(h), Some(d)) = (&hot, &deliver) {
            d.borrow_mut().finish(b, h);
            *cur = None;
            return;
        }
        if let Some(h) = &hot {
            if let (true, Some(j)) = (tw.measured_unload, *cur) {
                // Lever `a`: measured, its phase cancelled by Cliffords on the one-hot.
                onehot::unload_measured(b, h, r.angle_q, onehot::leaves(t), &ang, j);
                *cur = None;
                return;
            }
            if !fault {
                match (emb, *cur) {
                    (Some(e), Some(c)) => onehot::embed(b, t, h, r.angle_q, &e.piv[c], c, false),
                    (Some(_), None) => {}
                    (None, _) => {
                        let nl = onehot::leaves(t);
                        onehot::transition_by(b, h, r.angle_q, nl, &ang, *cur, None);
                    }
                }
            }
            for &q in r.angle_q {
                onehot::reset_keep(b, q);
            }
            *cur = None;
        }
    };
    z_s(b);
    pi_phase(b);
    rotate(b, true, &mut cur);
    z_s(b);
    // `q<b>_1.<c>`: cut the held span to `c` dimensions across the Majorana.
    if let (Some(h), Some(d)) = (&hot, &deliver) {
        d.borrow_mut().park(b, h);
    }
    // The split one-hot (levers `v`, `w`) keeps rotation 0's angle loaded across the Majorana: its
    // unload and reload would cost the group correction twice.
    // Lever `P`: with `a`, keep it loaded anyway (the Majorana stage has room).
    if deliver.is_none() && (tw.hot_groups == 0 || (tw.measured_unload && !tw.hold_maj)) {
        unload(b, &mut cur, false);
    }
    // Lever `.h<k>`: `k` of rotation 0's register bits leave across the Majorana
    // (measured, Clifford fixup) and are reloaded after it (a transition from 0 on those bits).
    let drop_bits: Vec<usize> = match (&hot, tw.maj_drop) {
        (Some(h), k) if k > 0 => {
            assert!(
                tw.hold_maj && deliver.is_none() && emb.is_none() && cur == Some(0),
                "lever .h needs U (rotation 0 held across the Majorana)"
            );
            let w0 = t.widths[0] - usize::from(pe[0]);
            let nl = onehot::leaves(t);
            // Reload cost proxy per bit: the group bits whose difference from group 0 has it.
            let cost = |k: usize| -> usize {
                (1..=h.g.len())
                    .filter(|&i| {
                        (0..h.e1).any(|c| {
                            let a = h.leaf(0, c, nl).map_or(0, |e| ang(e, 0));
                            h.leaf(i, c, nl)
                                .is_some_and(|e| (a ^ ang(e, 0)) >> k & 1 == 1)
                        })
                    })
                    .count()
            };
            let mut bits: Vec<usize> = (0..w0).collect();
            bits.sort_by_key(|&k| (cost(k), k));
            bits.truncate(usize::from(k).min(w0));
            bits.sort_unstable();
            bits
        }
        _ => Vec::new(),
    };
    let drop_q: Vec<Qubit> = drop_bits.iter().map(|&k| r.angle_q[k]).collect();
    let drop_ang = |e: usize, j: usize| -> u64 {
        let a = ang(e, j);
        drop_bits
            .iter()
            .enumerate()
            .fold(0, |acc, (i, &k)| acc | (a >> k & 1) << i)
    };
    if let (Some(h), false) = (&hot, drop_q.is_empty()) {
        onehot::unload_measured(b, h, &drop_q, onehot::leaves(t), &drop_ang, 0);
    }
    led.stage(b, "copy: SELECT V^dagger");
    // id = [b = B]: an AND of b's literals (derive_id), held only around the Majorana. On a
    // one-body lane b holds r mod 2^k_i here and id may be 1, but it is only read with en_sq.
    let lits: Vec<Qubit> = bsel.clone();
    let flip = |b: &mut Builder| {
        for (j, &q) in lits.iter().enumerate() {
            if spec_b >> j & 1 == 0 {
                b.x(q);
            }
        }
    };
    let ob_fan = |b: &mut Builder| {
        if let Some(h) = &hot {
            onehot::transition_by(b, h, &[r.is_ob], n_leaves, &ob_value, None, Some(0));
        }
    };
    let (id, chain) = if let Some(idp) = held_id {
        ob_fan(b);
        (idp, None)
    } else if tw.derive_id {
        flip(b);
        let (acc, mut chain) = all_ones(b, &lits);
        if tw.majorana_cut {
            // Lever `y`: keep only `id` (the chain's top); erase the rest now, top first.
            let top = chain.pop();
            erase_all_ones(b, chain);
            chain = top.into_iter().collect();
        }
        (acc, Some(chain))
    } else {
        (main[k_i + 1], None)
    };
    if let Some(bit) = pass_bit {
        // Lever `O`: `pos_e` from the one-hot (a fan-out in, an X measurement out).
        let pos_value = |e: usize, _j: usize| -> u64 {
            let rb = t.spec.r * t.spec.b;
            u64::from(e >= rb && t.spec.e[e - rb] >= 0.0)
        };
        let pe = match (&hot, tw.pos_from_hot) {
            (Some(h), true) => {
                let q = b.alloc();
                if !onehot::fault_is(43) {
                    onehot::transition_by(b, h, &[q], n_leaves, &pos_value, None, Some(0));
                }
                Some(q)
            }
            _ => None,
        };
        majorana_cut(b, r, neg, id, bit, pe.unwrap_or(r.pos_e));
        if let (Some(h), Some(q)) = (&hot, pe) {
            onehot::unload_measured(b, h, &[q], n_leaves, &pos_value, 0);
            b.free(q);
        }
    } else {
        let neg = neg.expect("lever S needs y");
        if p.swap {
            super::inner::majorana_spin0(b, r, neg, id);
        } else {
            super::inner::majorana(b, r, neg, id);
        }
    }
    if let Some(chain) = chain {
        if tw.majorana_cut && !chain.is_empty() {
            // `id` is erased by an X measurement; where it reads 1, `(-1)^id` is applied again
            // (an AND chain over the literals, from and back to a pool, erased by measurement).
            let m = b.hmr(id);
            let mut pool = narrow::Pool::default();
            b.push_condition(m);
            if !onehot::fault_is(11) {
                narrow::and_phase(b, &lits, &mut pool);
            }
            b.pop_condition();
            pool.close(b);
        } else {
            erase_all_ones(b, chain);
        }
        flip(b);
    }
    if held_id.is_some() {
        match (&hot, tw.measured_ck) {
            (Some(h), true) => {
                onehot::unload_measured(b, h, &[r.is_ob], n_leaves, &ob_value, 0);
            }
            _ => {
                ob_fan(b);
                onehot::reset_keep(b, r.is_ob);
            }
        }
    }
    led.stage(b, "copy: SELECT Majorana and controls");
    if let (Some(h), false) = (&hot, drop_q.is_empty()) {
        if !onehot::fault_is(124) {
            onehot::transition_by(b, h, &drop_q, onehot::leaves(t), &drop_ang, None, Some(0));
        }
    }
    rotate(b, false, &mut cur);
    unload(b, &mut cur, onehot::fault_is(3));
    pi_phase(b);
    let erase_id = |b: &mut Builder, idp: Qubit| {
        // `id'` erased by an X measurement; where it reads 1, `(-1)^(id')` again by an AND
        // chain over its literals (from and back to a pool, erased by measurement).
        let m = b.hmr(idp);
        let mut pool = narrow::Pool::default();
        flip7(b);
        b.push_condition(m);
        if !onehot::fault_is(45) {
            narrow::and_phase(b, &lits7, &mut pool);
        }
        b.pop_condition();
        pool.close(b);
        flip7(b);
    };
    if let (Some(idp), Some(h), true) = (held_id, &hot, pre_id.is_some()) {
        ck_fan(b, h, idp);
    } else if let (Some(idp), Some(h)) = (held_id, &hot) {
        ck_fan(b, h, idp);
        // `id'` erased by an X measurement; where it reads 1, `(-1)^(id')` again by an AND
        // chain over its literals (from and back to a pool, erased by measurement).
        let m = b.hmr(idp);
        let mut pool = narrow::Pool::default();
        flip7(b);
        b.push_condition(m);
        narrow::and_phase(b, &lits7, &mut pool);
        b.pop_condition();
        pool.close(b);
        flip7(b);
    }
    led.stage(b, "copy: SELECT V");
    if p.swap && sel.is_none() {
        sel = Some(super::inner::spin_select(b, r));
    }
    if let Some(g) = sel {
        b.spin_swap(g);
        super::inner::spin_unselect(b, r, g);
        led.stage(b, "copy: SELECT controlled spin swap F^s");
    }

    // 4. RPREP^dagger.
    let asplit = split_for(tw.hot_erase, idx.len(), e);
    if let Some(h) = hot {
        if h.rec.is_some() {
            onehot::erase_classes_with(b, &bsel, &r.hi, h, tw.factor_erase);
        } else {
            onehot::erase_keep(b, t, &idx, e, h, asplit, tw.in_range, emb.is_some());
        }
        if let Some(idp) = pre_id {
            erase_id(b, idp);
        }
    } else {
        let ma: Vec<Bit> = r.angle_q.iter().map(|&q| hmr_keep(b, q)).collect();
        qroam::fixup(b, &idx, e, &ma, &angles, asplit);
    }
    if !r.lo.is_empty() {
        for (&o, &q) in r.lo.iter().zip(&bsel) {
            b.cx(o, q);
        }
    }
    led.stage(b, "copy: RPREP^dagger angle erasure");

    // 5. Inner PREPARE^dagger.
    let isplit = if start > 0 {
        qroam::best_split_range(index.len(), start, limit, tw.hot_erase)
    } else {
        split_for(tw.hot_erase, index.len(), limit)
    };
    if tw.measured_undo {
        // Lever `R`: the comparison again (keep is held), and `Z` for the early outcome.
        let (lt, cmp) = match early_lt {
            Some(m) => {
                let l = less_than(b, &r.draw, &keep);
                if !onehot::fault_is(44) {
                    b.z_if(l, m);
                }
                (
                    l,
                    Cmp {
                        lt: l,
                        ladder: Vec::new(),
                    },
                )
            }
            None => (lt, cmp),
        };
        if tw.index_host {
            // Lever `_`: the alt register again (a fresh copy of the unchosen index), then the
            // inner index back to `i` (`bsel` holds the chosen index again here).
            alt = b.alloc_n(k_i);
            for (&a, &i) in alt.iter().zip(&r.index) {
                b.cx(i, a);
            }
            if !onehot::fault_is(161) {
                index_swap(b, lt, &bsel, &alt, &r.index);
            }
        }
        let mm = undo_measured(b, lt, &main, &alt);
        let (mb, mf) = mm.split_at(k_i);
        if r.lo.is_empty() {
            // b held b' ^ is_ob q_low when measured.
            for (j, &m) in mb.iter().enumerate() {
                b.cz_if(r.is_ob, r.q[j], m);
            }
        }
        let keep_word = |x: u64| narrow::low_bits(&words(x), mu);
        let a2 = p.inner_a.min(k_i);
        // Lever `~`: `lt` leaves by X measurement now; the gated re-read that cancels its
        // outcome's phase runs once the item one-hot is written again (below).
        let lt_m: Option<Bit> = hot_keep.is_some().then(|| b.hmr(cmp.lt));
        let reread: Vec<Bit> = if hot_keep.is_some() {
            Vec::new()
        } else if early_keep.is_some() {
            let fault = if onehot::fault_is(9) {
                narrow::Fault::NoPhase
            } else {
                narrow::Fault::None
            };
            let rr = narrow::Reread { a: a2, fix: None };
            narrow::gated_lt_erase_from(b, lt, &r.draw, &index, start, limit, &keep_word, rr, fault)
        } else {
            uncompare(b, &r.draw, &keep, cmp, tw.gated);
            Vec::new()
        };
        led.stage(b, "copy: inner alt swap (measured) and keep test undone");
        let mk: Vec<Bit> = early_keep
            .or(hot_keep)
            .unwrap_or_else(|| keep.iter().map(|&q| b.hmr(q)).collect());
        let mut mal: Vec<Bit> = alt.iter().map(|&q| b.hmr(q)).collect();
        // One-body lanes (index past the table): alt' held i ^ (is_ob q_low) in its b slots.
        for (j, &m) in mal.iter().take(k_i).enumerate() {
            // Lever `t`: the `i` part of every lane's alt slot (the table is rooted at the
            // control, so it no longer covers it on the square lanes).
            if sp {
                if !onehot::fault_is(95) {
                    b.z_if(r.index[j], m);
                }
            } else {
                b.cz_if(r.is_ob, r.index[j], m);
            }
            if r.lo.is_empty() {
                b.cz_if(r.is_ob, r.q[j], m);
            }
        }
        // Lever `S`: the sign slots were measured at PREPARE, holding the plain table bits.
        let own_slot: Vec<Bit> = match (early_signs, early_delta) {
            (Some((m_own, m_alt)), _) => {
                mal.push(m_alt);
                vec![m_own]
            }
            (None, Some(m_d)) => vec![m_d],
            (None, None) => mf.to_vec(),
        };
        let out_bits: Vec<Bit> = mk.into_iter().chain(own_slot).chain(mal).collect();
        let wbits = t.inner_word_bits();
        let mask = (1u64 << k_i) - 1;
        let signs_early = early_signs.is_some();
        let content = |x: u64| -> Word {
            let wd = words(x);
            let own_f = get(&wd, mu, fb);
            let alt_b = get(&wd, mu + fb, k_i);
            let alt_f = get(&wd, mu + fb + k_i, fb);
            let mut c = word(wbits);
            put(&mut c, 0, mu, get(&wd, 0, mu));
            put(&mut c, mu + fb, k_i, (x & mask) ^ alt_b);
            if signs_early {
                put(&mut c, mu, fb, own_f);
                put(&mut c, mu + fb + k_i, fb, alt_f);
            } else {
                put(&mut c, mu, fb, alt_f);
                put(&mut c, mu + fb + k_i, fb, own_f ^ alt_f);
            }
            c
        };
        // The extra outcomes: `b`'s, then (lever `j`) the gated re-read's (0 where it did not
        // run; `narrow::read_content`'s layout).
        let nr = reread.len();
        let extra = |x: u64| -> Word {
            let mut c = word(k_i + nr);
            put(&mut c, 0, k_i, get(&words(x), mu + fb, k_i));
            if nr > 0 && !onehot::fault_is(10) {
                let rc = narrow::read_content(x, a2, mu, limit, &keep_word);
                for j in 0..nr {
                    put(&mut c, k_i + j, 1, get(&rc, j, 1));
                }
            }
            c
        };
        let mut extra_bits = mb.to_vec();
        extra_bits.extend(reread);
        // Lever `~`: the keep of `(item, i)` (the read's low `mu` bits; rooted with `t`).
        let kdata = |item: u64, i: u64| -> Word {
            let wd = if sp {
                titem(item, i)
            } else {
                words(item << k_i | i)
            };
            narrow::low_bits(&wd, mu)
        };
        // Lever `~`: where `lt`'s outcome is 1, `keep(item, i)` again from the one-hot, the
        // comparison's phase, the re-read measured (`itemhot::gated_lt_erase_hot`). Its
        // outcomes join the erasure pass as one more part with the keep's data.
        let gated_hot = |b: &mut Builder, h: &ItemHot| -> Vec<Bit> {
            let Some(m) = lt_m else {
                return Vec::new();
            };
            let fault = if onehot::fault_is(201) {
                narrow::Fault::NoPhase
            } else if onehot::fault_is(202) {
                narrow::Fault::FlipOutcome
            } else {
                narrow::Fault::None
            };
            let rows = itemhot::rows_needed(h, 1 << k_i, &[&kdata]);
            let root = sp.then_some(ctl);
            let bits =
                itemhot::gated_lt_erase_hot(b, h, m, &r.draw, &r.index, rows, &kdata, root, fault);
            if onehot::fault_is(206) {
                // (Mutant 206: the re-read's outcomes left out of the erasure pass.)
                return Vec::new();
            }
            bits
        };
        if let (true, Some(h)) = (sp, ih) {
            // Lever `t`: what was measured is the read's word (the alt slot's `i` part is
            // already cancelled), every part 0 off the control-1 square lanes.
            let hk_bits = if onehot::fault_is(207) {
                // (Mutant 207: the re-read before the one-hot is written again.)
                let hk = gated_hot(b, h);
                itemhot::write(b, h, &r.q);
                hk
            } else {
                itemhot::write(b, h, &r.q);
                gated_hot(b, h)
            };
            let edata = |item: u64, i: u64| extra(item << k_i | i);
            let root = (!onehot::fault_is(94)).then_some(ctl);
            if tw.pad_offset {
                // Lever `+`: both parts offset by their padding content; the offsets' phase is
                // applied under the control from the one-hot (Cliffords and one AND).
                let top = (1u64 << k_i) - 1;
                let c1 = |item: u64| titem(item, top);
                let c2 = |item: u64| edata(item, top);
                let xor =
                    |a: Word, c: Word| -> Word { a.iter().zip(&c).map(|(x, y)| x ^ y).collect() };
                let d1 = |item: u64, i: u64| xor(titem(item, i), c1(item));
                let d2 = |item: u64, i: u64| xor(edata(item, i), c2(item));
                // Lever `~`: the re-read's part, offset like the others.
                let c3 = |item: u64| kdata(item, top);
                let d3 = |item: u64, i: u64| xor(kdata(item, i), c3(item));
                let n_rows = if hk_bits.is_empty() {
                    itemhot::rows_needed(h, 1 << k_i, &[&d1, &d2])
                } else {
                    itemhot::rows_needed(h, 1 << k_i, &[&d1, &d2, &d3])
                };
                let mut parts: Vec<itemhot::Part<'_, '_>> =
                    vec![(&out_bits, &d1), (&extra_bits, &d2)];
                let mut offs: Vec<itemhot::ItemPart<'_, '_>> =
                    vec![(&out_bits, &c1), (&extra_bits, &c2)];
                if !hk_bits.is_empty() {
                    parts.push((&hk_bits, &d3));
                    offs.push((&hk_bits, &c3));
                }
                phase_fn(b, h, &r.index, n_rows, &parts, root);
                if !onehot::fault_is(105) {
                    itemhot::fan_rooted(b, h, ctl, &[], &|_| word(1), None, &offs);
                }
            } else {
                let mut parts: Vec<itemhot::Part<'_, '_>> =
                    vec![(&out_bits, &titem), (&extra_bits, &edata)];
                if !hk_bits.is_empty() {
                    parts.push((&hk_bits, &kdata));
                }
                phase_fn(b, h, &r.index, 1 << k_i, &parts, root);
            }
        } else if let Some(h) = ih {
            let hk_bits = if onehot::fault_is(207) {
                let hk = gated_hot(b, h);
                itemhot::write(b, h, &r.q);
                hk
            } else {
                itemhot::write(b, h, &r.q);
                gated_hot(b, h)
            };
            let cdata = |item: u64, i: u64| content(item << k_i | i);
            let edata = |item: u64, i: u64| extra(item << k_i | i);
            let mut parts: Vec<itemhot::Part<'_, '_>> =
                vec![(&out_bits, &cdata), (&extra_bits, &edata)];
            if !hk_bits.is_empty() {
                parts.push((&hk_bits, &kdata));
            }
            phase_fn(b, h, &r.index, 1 << k_i, &parts, None);
        } else if tw.paired_erase {
            // Lever `F`: the read had no junk (`P`), so the erasure is the phase of the
            // measured outcomes alone, by the paired phase lookup.
            assert!(tw.paired_lookup, "lever F needs P");
            let nw = out_bits.len();
            let ne = extra_bits.len();
            let bits: Vec<Bit> = out_bits.iter().chain(&extra_bits).copied().collect();
            let both = |x: u64| -> Word {
                let mut c = word(nw + ne);
                // Lever `t` (`^`): what was measured is the read's word, 0 off the control-1
                // square lanes (the alt slot's `i` part is already cancelled).
                let (cw, ew) = (if sp { tword(x) } else { content(x) }, extra(x));
                put(&mut c, 0, nw, get(&cw, 0, nw));
                put(&mut c, nw, ne, get(&ew, 0, ne));
                c
            };
            let _ = read;
            let root = (sp && !onehot::fault_is(94)).then_some(ctl);
            super::paired::phase_paired_rooted(b, &index, start, limit, &bits, &both, root);
        } else {
            super::range::inner_erase_parts(
                b,
                &index,
                read.expect("the QROAM read"),
                &words,
                out_bits,
                &content,
                extra_bits,
                &extra,
                isplit,
            );
        }
    } else {
        choose_alt(b, lt, &main, &alt);
        uncompare(b, &r.draw, &keep, cmp, tw.gated);
        for (&i, &q) in r.index.iter().zip(&bsel) {
            b.cx(i, q);
        }
        bsel.into_iter().for_each(|q| b.free(q));
        led.stage(b, "copy: inner alt swap and keep test undone");
        super::range::inner_erase_split_by(
            b,
            &index,
            read.expect("lever H needs m"),
            &words,
            isplit,
        );
    }
    match pass_bit {
        Some(bit) if !onehot::fault_is(12) => {
            let mut op = crate::circuit::Op::new(crate::circuit::OperationType::BitInvert);
            op.c_target = bit.0;
            b.emit(op);
        }
        Some(_) => {}
        None => b.x(r.pass),
    }
    led.stage(b, "copy: inner alias read erased");
}

/// `inner::majorana_spin0` for lever `y` (`majorana_cut`; same
/// operator): the enables are made here, one at a time (`en_sq = control AND NOT is_ob` for the
/// square part, then `en_ob = control AND is_ob` for the one-body part), and the pass flag is
/// the classical `pass`.
fn majorana_cut(
    b: &mut Builder,
    r: &InnerRegs<'_>,
    neg: Option<Qubit>,
    id: Qubit,
    pass: Bit,
    pos_e: Qubit,
) {
    let control = b.control();
    let z0 = b.system(0);
    let is_ob = r.is_ob;
    let en_sq = b.alloc();
    b.x(is_ob);
    b.ccx(control, is_ob, en_sq);
    b.x(is_ob);
    if let Some(neg) = neg {
        b.cz(en_sq, neg);
    }
    let pz = b.alloc();
    b.x(id);
    b.ccx(en_sq, id, pz);
    b.x(id);
    b.cz(pz, z0);
    erase_and(b, en_sq, id, pz, true);
    erase_and(b, control, is_ob, en_sq, true);
    let en_ob = b.alloc();
    b.ccx(control, is_ob, en_ob);
    let x = r.index[0];
    let ex = b.alloc();
    b.ccx(en_ob, x, ex);
    b.cz(ex, z0);
    b.cx(en_ob, z0);
    b.cz(ex, pos_e);
    b.z_if(ex, pass);
    erase_and(b, en_ob, x, ex, false);
    erase_and(b, control, is_ob, en_ob, false);
}

/// Lever `P`: the rotations whose `pi` bit is applied at the chain's edge, `Z_p Z_q` on their
/// modes (both spins): those that need the top bit of a one-precision spec and whose modes no
/// later rotation touches (so `Z_p Z_q`, which commutes with their own Givens, commutes with
/// every rotation after them in `V` and before them in `V^dagger`).
pub(super) fn pi_edge_set(t: &SaTables<'_>, on: bool) -> Vec<bool> {
    let beta = usize::from(t.spec.beta);
    (0..t.modes.len())
        .map(|j| {
            let (p, q) = t.modes[j];
            on && t.spec.widths.is_none()
                && t.widths[j] == beta
                && beta > 1
                && t.modes[j + 1..]
                    .iter()
                    .all(|&(a, c)| a != p && a != q && c != p && c != q)
        })
        .collect()
}
