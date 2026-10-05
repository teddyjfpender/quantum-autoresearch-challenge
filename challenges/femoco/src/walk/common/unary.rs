//! Unary iteration (Babbush et al. 2018, arXiv:1805.03662, Sec. III.A, Fig. 7).
//!
//! Visits every value `v` of a little-endian index register below `limit`, in increasing
//! order, and hands the caller a qubit that is 1 exactly on the lanes where the (optional)
//! control is 1 and the index equals `v`. Each internal node of the binary tree over the index
//! bits costs one AND (a Toffoli) to enter its left child, a CNOT to move to its right child,
//! and a measurement (`Hmr` plus a conditioned `CZ`, no Toffoli) to erase it. Without a
//! control the top level needs no AND: the top index bit itself (or its negation) is the
//! child's indicator.
use crate::circuit::{Builder, Qubit};

/// Called at each visited value with the builder, the value and its indicator qubit.
pub type Leaf<'a> = dyn FnMut(&mut Builder, u64, Qubit) + 'a;

/// Iterates over `index` (little-endian) for values `0..limit`.
///
/// Lanes whose index is `>= limit` get no indicator; callers either cover every value
/// (`limit = 2^len`) or guarantee the index is in range on lanes where `ctl` is 1.
///
/// # Panics
/// If `index` is empty and there is no control (there is nothing to iterate on).
pub fn iterate(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    limit: u64,
    leaf: &mut Leaf<'_>,
) {
    assert!(ctl.is_some() || !index.is_empty(), "nothing to iterate on");
    node(b, ctl, index, 0, limit, leaf);
}

fn node(
    b: &mut Builder,
    ctl: Option<Qubit>,
    index: &[Qubit],
    base: u64,
    limit: u64,
    leaf: &mut Leaf<'_>,
) {
    let Some((&bit, low)) = index.split_last() else {
        leaf(b, base, ctl.expect("a leaf needs an indicator"));
        return;
    };
    let half = 1u64 << low.len();
    let right = base + half < limit;
    let Some(c) = ctl else {
        // Top level without a control: the indicator of the left half is NOT bit, of the
        // right half bit itself.
        b.x(bit);
        node(b, Some(bit), low, base, limit, leaf);
        b.x(bit);
        if right {
            node(b, Some(bit), low, base + half, limit, leaf);
        }
        return;
    };
    // child = c AND NOT bit.
    let child = b.alloc();
    b.x(bit);
    b.ccx(c, bit, child);
    b.x(bit);
    node(b, Some(child), low, base, limit, leaf);
    if right {
        // child ^= c turns (c AND NOT bit) into (c AND bit).
        b.cx(c, child);
        node(b, Some(child), low, base + half, limit, leaf);
        erase_and(b, c, bit, child, false);
    } else {
        erase_and(b, c, bit, child, true);
    }
}

/// Erases `t = a AND (NOT) c` by X-basis measurement: an outcome of 1 left a `-1` on the lanes
/// where `t` was 1, which the conditioned `CZ(a, c)` cancels (Gidney 2018, Fig. 3).
pub fn erase_and(b: &mut Builder, a: Qubit, c: Qubit, t: Qubit, negated: bool) {
    let m = b.hmr(t);
    if negated {
        b.x(c);
    }
    b.cz_if(a, c, m);
    if negated {
        b.x(c);
    }
}
