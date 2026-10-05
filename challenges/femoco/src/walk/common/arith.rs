//! Small reversible arithmetic: controlled swaps, the keep comparison and equality tests.
use super::unary::erase_and;
use crate::circuit::{Builder, Qubit};

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
pub(crate) fn carries(b: &mut Builder, na: &[Qubit], bb: &[Qubit], n: usize) -> Vec<Qubit> {
    let mut cs: Vec<Qubit> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = i.checked_sub(1).map(|j| cs[j]);
        cs.push(carry(b, na[i], bb[i], prev));
    }
    cs
}

pub(crate) fn erase_carries(b: &mut Builder, na: &[Qubit], bb: &[Qubit], cs: &[Qubit]) {
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

/// [`less_than`] that keeps every carry live: returns them all, the last being `[a < b]`.
/// `n` Toffolis; [`erase_less_than_keep`] erases them by measurement with none, so a comparison
/// that is undone later costs `n` instead of `2n - 1`, for `n - 1` qubits held in between.
pub fn less_than_keep(b: &mut Builder, a: &[Qubit], bb: &[Qubit]) -> Vec<Qubit> {
    assert!(
        !a.is_empty() && a.len() == bb.len(),
        "bad comparator widths"
    );
    a.iter().for_each(|&q| b.x(q));
    let cs = carries(b, a, bb, a.len());
    a.iter().for_each(|&q| b.x(q));
    cs
}

/// Erases [`less_than_keep`]'s carries (on unchanged `a`, `b`) by measurement: no Toffolis.
pub fn erase_less_than_keep(b: &mut Builder, a: &[Qubit], bb: &[Qubit], cs: &[Qubit]) {
    a.iter().for_each(|&q| b.x(q));
    erase_carries(b, a, bb, cs);
    a.iter().for_each(|&q| b.x(q));
}

/// The AND of every qubit in `qs` as a chain of Toffolis. Returns the result qubit and the
/// chain's `(a, c, t)` triples, which [`erase_all_ones`] erases.
pub fn all_ones(b: &mut Builder, qs: &[Qubit]) -> (Qubit, Vec<(Qubit, Qubit, Qubit)>) {
    let mut acc = qs[0];
    let mut chain = Vec::new();
    for &q in &qs[1..] {
        let t = b.alloc();
        b.ccx(acc, q, t);
        chain.push((acc, q, t));
        acc = t;
    }
    (acc, chain)
}

/// Erases the chain of [`all_ones`] by measurement, top first.
pub fn erase_all_ones(b: &mut Builder, chain: Vec<(Qubit, Qubit, Qubit)>) {
    for (a, c, t) in chain.into_iter().rev() {
        erase_and(b, a, c, t, false);
    }
}
