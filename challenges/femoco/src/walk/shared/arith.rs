//! Reversible arithmetic on little-endian registers, with every AND erased by measurement
//! (Gidney 2018, arXiv:1709.06648, Fig. 3), so an `n`-bit addition costs `n - 1` Toffolis.
//!
//! | function | effect | Toffolis |
//! | --- | --- | --- |
//! | [`add_into`] | `y += x mod 2^|y|` (`|x| <= |y|`) | `|y| - 1` |
//! | [`sub_from`] | `y -= x mod 2^|y|` | `|y| - 1` |
//! | [`ctrl_add`], [`ctrl_sub`] | `y +-= c x` | `|x| + |y| - 1` |
//! | [`less_than`] | fresh `[a < b]` (equal widths) | `n` |
//! | [`unless_than`] | erases a [`less_than`] result | `n - 1` |
//! | [`div_small`] | `r -> r mod d`, fresh `q = r div d` | `q_w (3 |d| + 2)` |
//! | [`div_small_undo`] | inverse of [`div_small`] | `q_w (3 |d| + 1)` |
//! | [`divmod_const`] | `x -> x mod m` in place, fresh `q = x div m` | as `div_small` |
//! | [`select_index`] | fresh `p = c ? x : y` | `|p|` (erased free by [`unselect_index`]) |
use super::unary::erase_and;
use crate::circuit::{Builder, Qubit};

/// Bits needed to hold every value in `0..=v`.
#[must_use]
pub fn bits_for_value(v: u64) -> usize {
    (u64::BITS - v.leading_zeros()).max(1) as usize
}

/// `y += x` modulo `2^|y|`, with `x` zero-extended (`|x| <= |y|`). `x` is unchanged.
///
/// Carries `c_{i+1} = MAJ(x_i, y_i, c_i)` are each one AND (using
/// `MAJ(x, y, c) = c ^ ((x ^ c) & (y ^ c))`); the top sum needs no carry out, and on the way
/// back every carry is erased by measurement while the sum bits are written.
///
/// # Panics
/// If `x` is wider than `y`.
pub fn add_into(b: &mut Builder, x: &[Qubit], y: &[Qubit]) {
    assert!(x.len() <= y.len(), "add_into: x wider than y");
    let n = y.len();
    if x.is_empty() || n == 0 {
        return;
    }
    let xb = |i: usize| x.get(i).copied();
    // c[i] = carry into bit i (c[0] = none).
    let mut c: Vec<Option<Qubit>> = vec![None; n];
    for i in 0..n - 1 {
        c[i + 1] = match (xb(i), c[i]) {
            (Some(xi), None) => {
                let t = b.alloc();
                b.ccx(xi, y[i], t);
                Some(t)
            }
            (Some(xi), Some(ci)) => {
                b.cx(ci, xi);
                b.cx(ci, y[i]);
                let t = b.alloc();
                b.ccx(xi, y[i], t);
                b.cx(ci, t);
                Some(t)
            }
            (None, Some(ci)) => {
                let t = b.alloc();
                b.ccx(y[i], ci, t);
                Some(t)
            }
            (None, None) => None,
        };
    }
    // Top bit: y ^= x ^ c.
    if let Some(xt) = xb(n - 1) {
        b.cx(xt, y[n - 1]);
    }
    if let Some(ct) = c[n - 1] {
        b.cx(ct, y[n - 1]);
    }
    for i in (0..n - 1).rev() {
        let Some(t) = c[i + 1] else { continue };
        match (xb(i), c[i]) {
            (Some(xi), None) => {
                erase_and(b, xi, y[i], t, false);
                b.cx(xi, y[i]);
            }
            (Some(xi), Some(ci)) => {
                b.cx(ci, t);
                erase_and(b, xi, y[i], t, false);
                b.cx(ci, xi);
                b.cx(xi, y[i]);
            }
            (None, Some(ci)) => {
                erase_and(b, y[i], ci, t, false);
                b.cx(ci, y[i]);
            }
            (None, None) => unreachable!("a carry needs an input"),
        }
    }
}

/// `y -= x` modulo `2^|y|` (as `NOT(NOT y + x)`).
pub fn sub_from(b: &mut Builder, x: &[Qubit], y: &[Qubit]) {
    y.iter().for_each(|&q| b.x(q));
    add_into(b, x, y);
    y.iter().for_each(|&q| b.x(q));
}

/// `y += c x`: `x AND c` into a scratch register, added, then erased by measurement.
pub fn ctrl_add(b: &mut Builder, c: Qubit, x: &[Qubit], y: &[Qubit]) {
    let z = b.alloc_n(x.len());
    for (&xi, &zi) in x.iter().zip(&z) {
        b.ccx(c, xi, zi);
    }
    add_into(b, &z, y);
    for (&xi, &zi) in x.iter().zip(&z) {
        erase_and(b, c, xi, zi, false);
    }
}

/// `y -= c x`.
pub fn ctrl_sub(b: &mut Builder, c: Qubit, x: &[Qubit], y: &[Qubit]) {
    y.iter().for_each(|&q| b.x(q));
    ctrl_add(b, c, x, y);
    y.iter().for_each(|&q| b.x(q));
}

/// The comparison window of step `j` of a restoring division: `r[j..j + |d| + 1]`, padded with
/// fresh zero qubits past the top of `r`, and `d` padded by one zero qubit.
struct Window {
    win: Vec<Qubit>,
    dd: Vec<Qubit>,
    pad: Vec<Qubit>,
}

fn window(b: &mut Builder, r: &[Qubit], d: &[Qubit], j: usize) -> Window {
    let w = d.len() + 1;
    let top = (j + w).min(r.len());
    let mut win: Vec<Qubit> = r[j.min(r.len())..top].to_vec();
    let mut pad = b.alloc_n(w - win.len() + 1);
    let dpad = pad.pop().expect("one padding qubit for d");
    win.extend_from_slice(&pad);
    let mut dd = d.to_vec();
    dd.push(dpad);
    pad.push(dpad);
    Window { win, dd, pad }
}

fn drop_window(b: &mut Builder, w: Window) {
    w.pad.into_iter().for_each(|q| b.free(q));
}

/// Restoring division by a small register: on lanes where `r < d 2^q_w` and `d >= 1`, turns
/// `r` into `r mod d` in place and returns fresh `q_w` bits holding `r div d`.
///
/// Step `j` (from the top) compares `r >> j` with `d` over `|d| + 1` bits (the invariant
/// `r < d 2^(j + 1)` makes the higher bits zero) and subtracts `d 2^j` where it fits.
pub fn div_small(b: &mut Builder, r: &[Qubit], d: &[Qubit], q_w: usize) -> Vec<Qubit> {
    let mut q: Vec<Qubit> = Vec::with_capacity(q_w);
    for j in (0..q_w).rev() {
        let w = window(b, r, d, j);
        let lt = less_than(b, &w.win, &w.dd);
        b.x(lt);
        ctrl_sub(b, lt, &w.dd, &w.win);
        drop_window(b, w);
        q.push(lt);
    }
    q.reverse();
    q
}

/// Undoes [`div_small`]: `r` back to the dividend, `q` erased and freed.
pub fn div_small_undo(b: &mut Builder, r: &[Qubit], d: &[Qubit], q: Vec<Qubit>) {
    for (j, qj) in q.into_iter().enumerate() {
        let w = window(b, r, d, j);
        ctrl_add(b, qj, &w.dd, &w.win);
        b.x(qj);
        unless_than(b, &w.win, &w.dd, qj);
        drop_window(b, w);
    }
}

/// Divides the register `x` by the constant `m >= 2` in place: `x` becomes `x mod m` (its
/// bits above `bits_for_value(m - 1)` end at 0) and the returned fresh register holds
/// `x div m`. Returns `(q, r)` with `r` the low bits of `x`.
///
/// # Panics
/// If `m < 2`.
pub fn divmod_const(b: &mut Builder, x: &[Qubit], m: u64) -> (Vec<Qubit>, Vec<Qubit>) {
    assert!(m >= 2, "divmod_const needs m >= 2");
    let mw = bits_for_value(m);
    let q_w = (x.len() + 1).saturating_sub(mw).max(1);
    let d = super::lookup::load_const(b, mw, m);
    let q = div_small(b, x, &d, q_w);
    super::lookup::unload_const(b, d, m);
    let rw = bits_for_value(m - 1).min(x.len());
    (q, x[..rw].to_vec())
}

/// Undoes [`divmod_const`].
pub fn divmod_const_undo(b: &mut Builder, x: &[Qubit], m: u64, q: Vec<Qubit>, _r: Vec<Qubit>) {
    let mw = bits_for_value(m);
    let d = super::lookup::load_const(b, mw, m);
    div_small_undo(b, x, &d, q);
    super::lookup::unload_const(b, d, m);
}

/// A fresh register `p = c ? x : y` (equal widths): `p = y ^ (c AND (x ^ y))`, one Toffoli per
/// bit. `x` and `y` are unchanged.
pub fn select_index(b: &mut Builder, c: Qubit, x: &[Qubit], y: &[Qubit]) -> Vec<Qubit> {
    assert_eq!(x.len(), y.len(), "select_index widths");
    let p = b.alloc_n(x.len());
    for ((&xi, &yi), &pi) in x.iter().zip(y).zip(&p) {
        b.cx(yi, pi);
        b.cx(xi, yi);
        b.ccx(c, yi, pi);
        b.cx(xi, yi);
    }
    p
}

/// Erases a [`select_index`] result with no Toffoli: each bit is measured in the X basis, and
/// the phase `(-1)^(m (y ^ c (x ^ y)))` is Clifford (a conditioned `Z` on `y` and a conditioned
/// `CZ` between `c` and `x ^ y`).
pub fn unselect_index(b: &mut Builder, c: Qubit, x: &[Qubit], y: &[Qubit], p: Vec<Qubit>) {
    for ((&xi, &yi), pi) in x.iter().zip(y).zip(p) {
        let m = b.hmr(pi);
        b.z_if(yi, m);
        b.cx(xi, yi);
        b.cz_if(c, yi, m);
        b.cx(xi, yi);
    }
}

// The helpers below are verbatim copies of the baselines' (controlled swaps and the keep
// comparison), kept here so `shared/` stands alone.

/// Swaps `x` and `y` where `c` is 1 (Fredkin: CX, Toffoli, CX). Self-inverse.
pub fn cswap(b: &mut Builder, c: Qubit, x: Qubit, y: Qubit) {
    b.cx(y, x);
    b.ccx(c, x, y);
    b.cx(y, x);
}

/// Swaps registers `xs` and `ys` bitwise where `c` is 1. One Toffoli per bit.
pub fn cswap_reg(b: &mut Builder, c: Qubit, xs: &[Qubit], ys: &[Qubit]) {
    assert_eq!(xs.len(), ys.len(), "cswap of unequal registers");
    for (&x, &y) in xs.iter().zip(ys) {
        cswap(b, c, x, y);
    }
}

/// Carry `c_{i+1} = MAJ(x_i, y_i, c_i)` into a fresh ancilla, using the identity
/// `MAJ(x, y, c) = c XOR ((x XOR c) AND (y XOR c))` (Gidney 2018): one Toffoli.
fn carry(b: &mut Builder, x: Qubit, y: Qubit, prev: Option<Qubit>) -> Qubit {
    let t = b.alloc();
    match prev {
        None => b.ccx(x, y, t),
        Some(c) => {
            b.cx(c, x);
            b.cx(c, y);
            b.ccx(x, y, t);
            b.cx(c, t);
            b.cx(c, x);
            b.cx(c, y);
        }
    }
    t
}

/// Erases a carry made by [`carry`] (its `prev` must still hold) by measurement.
fn erase_carry(b: &mut Builder, x: Qubit, y: Qubit, prev: Option<Qubit>, t: Qubit) {
    match prev {
        None => erase_and(b, x, y, t, false),
        Some(c) => {
            b.cx(c, t);
            b.cx(c, x);
            b.cx(c, y);
            erase_and(b, x, y, t, false);
            b.cx(c, x);
            b.cx(c, y);
        }
    }
}

/// Carries `c_1 .. c_n` of `NOT a + b` (with `a` already negated in place).
fn carries(b: &mut Builder, na: &[Qubit], bb: &[Qubit], n: usize) -> Vec<Qubit> {
    let mut cs: Vec<Qubit> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = i.checked_sub(1).map(|j| cs[j]);
        cs.push(carry(b, na[i], bb[i], prev));
    }
    cs
}

fn erase_carries(b: &mut Builder, na: &[Qubit], bb: &[Qubit], cs: &[Qubit]) {
    for i in (0..cs.len()).rev() {
        let prev = i.checked_sub(1).map(|j| cs[j]);
        erase_carry(b, na[i], bb[i], prev, cs[i]);
    }
}

/// Returns a fresh qubit holding `[a < b]` for equal-width little-endian registers.
///
/// `a < b` exactly when `NOT a + b` (with `NOT a = 2^n - 1 - a`) carries out of `n` bits, so
/// the result is the top carry of that sum: `n` Toffolis, and the lower carries are erased by
/// measurement at once.
pub fn less_than(b: &mut Builder, a: &[Qubit], bb: &[Qubit]) -> Qubit {
    assert!(
        !a.is_empty() && a.len() == bb.len(),
        "bad comparator widths"
    );
    let n = a.len();
    a.iter().for_each(|&q| b.x(q));
    let cs = carries(b, a, bb, n);
    erase_carries(b, a, bb, &cs[..n - 1]);
    a.iter().for_each(|&q| b.x(q));
    cs[n - 1]
}

/// Erases a result of [`less_than`] on the same registers: recomputes the lower carries
/// (`n - 1` Toffolis) and erases all of them by measurement.
pub fn unless_than(b: &mut Builder, a: &[Qubit], bb: &[Qubit], lt: Qubit) {
    let n = a.len();
    a.iter().for_each(|&q| b.x(q));
    let mut cs = carries(b, a, bb, n - 1);
    cs.push(lt);
    erase_carries(b, a, bb, &cs);
    a.iter().for_each(|&q| b.x(q));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::shared::testsim::Sim;

    fn setup(bits: u32) -> (Builder, Vec<Qubit>) {
        let mut b = Builder::new(0);
        b.declare_uniform(bits);
        let u = (0..bits).map(|i| b.uniform(i)).collect();
        (b, u)
    }

    #[test]
    fn adders() {
        for (xw, yw) in [(3usize, 3usize), (2, 5), (1, 4), (4, 4), (5, 7)] {
            for x in 0..1u64 << xw {
                for y in 0..1u64 << yw {
                    for sub in [false, true] {
                        let (mut b, u) = setup((xw + yw) as u32);
                        let (xs, ys) = u.split_at(xw);
                        if sub {
                            sub_from(&mut b, xs, ys);
                        } else {
                            add_into(&mut b, xs, ys);
                        }
                        let mut sim = Sim::new(&b, x * 7 + y);
                        sim.set_uniform(x | y << xw);
                        sim.run(b.ops());
                        let mask = (1u64 << yw) - 1;
                        let want = if sub { y.wrapping_sub(x) } else { y + x } & mask;
                        assert_eq!(sim.read(ys), want, "{x} {y} sub {sub}");
                        assert_eq!(sim.read(xs), x);
                        sim.assert_clean();
                        assert_eq!(sim.toffolis, yw as u64 - 1);
                    }
                }
            }
        }
    }

    #[test]
    fn controlled_adders_and_select() {
        for x in 0..8u64 {
            for y in 0..16u64 {
                for c in 0..2u64 {
                    let (mut b, u) = setup(8);
                    let (xs, rest) = u.split_at(3);
                    let (ys, cs) = rest.split_at(4);
                    ctrl_sub(&mut b, cs[0], xs, ys);
                    let mut sim = Sim::new(&b, x + y);
                    sim.set_uniform(x | y << 3 | c << 7);
                    sim.run(b.ops());
                    assert_eq!(sim.read(ys), y.wrapping_sub(c * x) & 15);
                    sim.assert_clean();
                }
            }
        }
        for x in 0..8u64 {
            for y in 0..8u64 {
                for c in 0..2u64 {
                    let (mut b, u) = setup(7);
                    let p = select_index(&mut b, u[6], &u[0..3], &u[3..6]);
                    let mut sim = Sim::new(&b, 3);
                    sim.set_uniform(x | y << 3 | c << 6);
                    sim.run(b.ops());
                    assert_eq!(sim.read(&p), if c == 1 { x } else { y });
                    unselect_index(&mut b, u[6], &u[0..3], &u[3..6], p);
                    for seed in 0..4 {
                        let mut sim = Sim::new(&b, seed);
                        sim.set_uniform(x | y << 3 | c << 6);
                        sim.run(b.ops());
                        sim.assert_clean();
                    }
                }
            }
        }
    }

    #[test]
    fn division() {
        for dw in [1usize, 2, 3] {
            for d in 1..1u64 << dw {
                let q_w = 3;
                let rw = dw + q_w;
                for r in 0..(d << q_w).min(1 << rw) {
                    let (mut b, u) = setup((rw + dw) as u32);
                    let (rs, ds) = u.split_at(rw);
                    let q = div_small(&mut b, rs, ds, q_w);
                    let mut sim = Sim::new(&b, r);
                    sim.set_uniform(r | d << rw);
                    sim.run(b.ops());
                    assert_eq!(sim.read(&q), r / d, "r {r} d {d}");
                    assert_eq!(sim.read(rs), r % d);
                    div_small_undo(&mut b, rs, ds, q);
                    let mut sim = Sim::new(&b, r + 1);
                    sim.set_uniform(r | d << rw);
                    sim.run(b.ops());
                    assert_eq!(sim.read(rs), r);
                    sim.assert_clean();
                }
            }
        }
        for m in [2u64, 3, 5, 7] {
            for x in 0..64u64 {
                let (mut b, u) = setup(6);
                let (q, r) = divmod_const(&mut b, &u, m);
                let mut sim = Sim::new(&b, x);
                sim.set_uniform(x);
                sim.run(b.ops());
                assert_eq!(sim.read(&q), x / m);
                assert_eq!(sim.read(&r), x % m);
                assert_eq!(sim.read(&u), x % m);
                divmod_const_undo(&mut b, &u, m, q, r);
                let mut sim = Sim::new(&b, x);
                sim.set_uniform(x);
                sim.run(b.ops());
                assert_eq!(sim.read(&u), x);
                sim.assert_clean();
            }
        }
    }
}
