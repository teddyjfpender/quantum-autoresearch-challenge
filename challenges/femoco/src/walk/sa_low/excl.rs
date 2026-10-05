//! Exclusivity algebra: gadgets that turn products of mutually exclusive bits
//! into Cliffords. A register whose bits are never two at a time 1 (a one-hot, group bits, the
//! flags of one level of a unary iteration) collapses the Boolean ring: `x_i x_j = 0` for
//! `i != j` and `x_i^2 = x_i`. So for exclusive `x` and any `f, f'`,
//!
//! `(alpha . x ^ f)(beta . x ^ f') = sum_l x_l (alpha_l f' ^ beta_l f ^ alpha_l beta_l) ^ f f'`,
//!
//! i.e. one Toffoli gives every exclusive bit `l` a coefficient from `{0, f, f', 1 ^ f ^ f'}`.
//! With the leftover `f f'` known (a function of something the circuit can fan out or load), one
//! product carries two independent coefficient functions. Hence the **local bound**: a target
//! `sum_l x_l T_l` with `n` exclusive terms needs `n / 2` Toffolis per output bit (amortised over
//! output bits for odd `n`), and the constructions here meet it.
//!
//! - [`low_hot`] / [`low_unhot`]: the one-hot of a few index bits written in place (a tree of
//!   ANDs, `2^a - 2` Toffolis) and erased by measurement with `CZ` fixups (0 Toffolis).
//! - [`mux`]: the **exclusive multiplexer** `out ^= sum_{l >= 1} x_l R_l` for clean registers `R_l`
//!   whose pairwise bit products are classically known (the caller loads them into `out`, see
//!   [`mux_leftover`]): `ceil((n) / 2)` Toffolis per bit for `n` registers, `n / 2` with the
//!   bit-pair share for odd `n`. It replaces the swap network of a clean QROAM (`(lambda - 1) w`
//!   Toffolis; Motlagh and Pocrnic's SelectCopy, arXiv:2605.20334, copies one block per Toffoli and
//!   bit) by `(lambda - 1) w / 2` plus the low one-hot.
use crate::circuit::{Builder, Qubit};
use crate::walk::common::lookup::hmr_keep;
use crate::walk::shared::lookup::{word, Word};

/// Deliberate faults (tests only): 40 a `mux` pair product's register CNOT skipped, 41 the low
/// one-hot's measured erasure without its `CZ`.
fn fault_is(k: u8) -> bool {
    super::onehot::fault_is(k)
}

/// A one-hot of index bits written in place: `x[v] = [low = v]` for `v < 2^a`, and the splits
/// that built it (`(node, child, bit)`; a `bit` of `usize::MAX` marks the free first split).
pub struct LowHot {
    pub x: Vec<Qubit>,
    splits: Vec<(usize, usize, usize)>,
}

/// Writes the one-hot of `low` (little-endian, `a` bits) in place: a root set to 1, then one
/// split per node and bit from the top, `child = node AND bit`, `node ^= child`; the root's split
/// is a copy (`AND(1, bit) = bit`). `2^a - 2` Toffolis for `a >= 1`.
pub fn low_hot(b: &mut Builder, low: &[Qubit]) -> LowHot {
    let a = low.len();
    let lam = 1usize << a;
    let x = b.alloc_n(lam);
    b.x(x[0]);
    let mut splits = Vec::new();
    // Breadth-first by bit, top bit first: after the splits on bits a-1..j, x[v] for v a
    // multiple of 2^j holds [low >> j == v >> j].
    for j in (0..a).rev() {
        let step = 1usize << (j + 1);
        for node in (0..lam).step_by(step) {
            let child = node + (1 << j);
            if node == 0 && j == a - 1 {
                b.cx(low[j], x[child]);
                splits.push((node, child, usize::MAX));
            } else {
                b.ccx(x[node], low[j], x[child]);
                splits.push((node, child, j));
            }
            b.cx(x[child], x[node]);
        }
    }
    LowHot { x, splits }
}

/// Erases a [`low_hot`] register: the splits undone in reverse, each child measured and its
/// phase `(-1)^(m node bit)` cancelled by a conditioned `CZ(node, bit)`. Cliffords only.
pub fn low_unhot(b: &mut Builder, low: &[Qubit], h: LowHot) {
    let a = low.len();
    for &(node, child, j) in h.splits.iter().rev() {
        b.cx(h.x[child], h.x[node]);
        if j == usize::MAX {
            b.cx(low[a - 1], h.x[child]);
            continue;
        }
        let m = hmr_keep(b, h.x[child]);
        if !fault_is(41) {
            b.cz_if(h.x[node], low[j], m);
        }
    }
    b.x(h.x[0]);
    for q in h.x {
        b.free(q);
    }
}

/// Pairs, an optional shared triple and an optional lone register (see [`mux_groups`]).
pub type MuxGroups = (Vec<(usize, usize)>, Option<[usize; 3]>, Option<usize>);

/// The groups of a multiplexer over `n` registers `1..=n`: pairs, and the last three as a shared
/// triple when `n` is odd and at least 3 (a lone register when `n = 1`).
#[must_use]
pub fn mux_groups(n: usize) -> MuxGroups {
    if n == 1 {
        return (Vec::new(), None, Some(1));
    }
    let odd = n % 2 == 1;
    let np = if odd { (n - 3) / 2 } else { n / 2 };
    let pairs = (0..np).map(|p| (2 * p + 1, 2 * p + 2)).collect();
    let triple = odd.then(|| [n - 2, n - 1, n]);
    (pairs, triple, None)
}

/// The classical part of [`mux`]: the XOR of the products' left-over terms for register words
/// `regs[l]` (`regs[0]` unused), which the caller loads into `out` with the base word.
#[must_use]
pub fn mux_leftover(regs: &[Word], w: usize) -> Word {
    let n = regs.len() - 1;
    let bit = |l: usize, k: usize| regs[l][k / 64] >> (k % 64) & 1 == 1;
    let mut out = word(w);
    let mut flip = |k: usize, on: bool| {
        if on {
            out[k / 64] ^= 1 << (k % 64);
        }
    };
    let (pairs, triple, _) = mux_groups(n);
    for (a, c) in pairs {
        for k in 0..w {
            flip(k, bit(a, k) && bit(c, k));
        }
    }
    if let Some([a, c, d]) = triple {
        let mut k = 0;
        while k < w {
            if k + 1 < w {
                let (k1, k2) = (k, k + 1);
                let (a1, c1, d1) = (bit(a, k1), bit(c, k1), bit(d, k1));
                let (a2, c2, d2) = (bit(a, k2), bit(c, k2), bit(d, k2));
                let p1 = c1 && a2;
                flip(k1, p1);
                flip(k2, p1);
                flip(k1, (d1 ^ a2) && (a1 ^ a2));
                flip(k2, (d2 ^ a2) && (c1 ^ c2));
                k += 2;
            } else {
                flip(k, bit(a, k) && bit(c, k));
                k += 1;
            }
        }
    }
    out
}

/// The exclusive multiplexer: `out ^= sum_{l >= 1} x[l] regs[l]` plus the products' left-over
/// terms ([`mux_leftover`], which the caller must also XOR into `out` from the registers'
/// classical values, so that they cancel). `x` must be exclusive. Per output bit: one Toffoli per
/// pair `(x_a ^ R_c)(x_c ^ R_a) = x_a R_a ^ x_c R_c ^ R_a R_c`, and for an odd count a triple
/// shared over pairs of bits (`onehot::triple`'s algebra with registers in place of slot
/// parities), three Toffolis per two bits; a lone register costs one Toffoli per bit.
pub fn mux(b: &mut Builder, x: &[Qubit], regs: &[Vec<Qubit>], out: &[Qubit]) {
    let n = regs.len() - 1;
    let w = out.len();
    let (pairs, triple, lone) = mux_groups(n);
    for (a, c) in pairs {
        for k in 0..w {
            if !fault_is(40) {
                b.cx(x[a], regs[c][k]);
            }
            b.cx(x[c], regs[a][k]);
            b.ccx(regs[c][k], regs[a][k], out[k]);
            if !fault_is(40) {
                b.cx(x[a], regs[c][k]);
            }
            b.cx(x[c], regs[a][k]);
        }
    }
    if let Some(l) = lone {
        for k in 0..w {
            b.ccx(x[l], regs[l][k], out[k]);
        }
    }
    let Some([la, lc, ld]) = triple else {
        return;
    };
    let (a, c, d) = (x[la], x[lc], x[ld]);
    let (ra, rc, rd) = (&regs[la], &regs[lc], &regs[ld]);
    let mut k = 0;
    while k < w {
        if k + 1 < w {
            let (k1, k2) = (k, k + 1);
            // v1 = (a ^ d ^ C1)(c ^ A2) into both bits through a scratch qubit.
            let t = b.alloc();
            b.cx(d, a);
            b.cx(rc[k1], a);
            b.cx(ra[k2], c);
            b.ccx(a, c, t);
            b.cx(t, out[k1]);
            b.cx(t, out[k2]);
            let m = b.hmr(t);
            b.cz_if(a, c, m);
            b.cx(ra[k2], c);
            b.cx(rc[k1], a);
            b.cx(d, a);
            // v2 = (a ^ D1 ^ A2)(d ^ A1 ^ A2) into k1.
            b.cx(rd[k1], a);
            b.cx(ra[k2], a);
            b.cx(ra[k1], d);
            b.cx(ra[k2], d);
            b.ccx(a, d, out[k1]);
            b.cx(ra[k2], d);
            b.cx(ra[k1], d);
            b.cx(ra[k2], a);
            b.cx(rd[k1], a);
            // v3 = (c ^ D2 ^ A2)(d ^ C1 ^ C2) into k2.
            b.cx(rd[k2], c);
            b.cx(ra[k2], c);
            b.cx(rc[k1], d);
            b.cx(rc[k2], d);
            b.ccx(c, d, out[k2]);
            b.cx(rc[k2], d);
            b.cx(rc[k1], d);
            b.cx(ra[k2], c);
            b.cx(rd[k2], c);
            k += 2;
        } else {
            // (a ^ C)(c ^ A) and the single d.
            b.cx(rc[k], a);
            b.cx(ra[k], c);
            b.ccx(a, c, out[k]);
            b.cx(ra[k], c);
            b.cx(rc[k], a);
            b.ccx(d, rd[k], out[k]);
            k += 1;
        }
    }
}

/// Toffolis of [`mux`] over `n` registers of `w` bits.
#[must_use]
pub fn mux_cost(n: usize, w: usize) -> usize {
    let (pairs, triple, lone) = mux_groups(n);
    pairs.len() * w + lone.map_or(0, |_| w) + triple.map_or(0, |_| 3 * (w / 2) + 2 * (w % 2))
}

#[cfg(test)]
mod tests {
    //! Exhaustive: every low value and random register contents, the multiplexer gives the
    //! selected register, the low one-hot is exact and its erasure clean; the mutants are caught.
    use super::*;
    use crate::walk::sa_low::onehot::FAULT;
    use crate::walk::shared::testsim::Sim;

    fn run(a: usize, w: usize, regs: &[u64], low: u64, fault: u8, seed: u64) -> (u64, bool, u64) {
        let lam = 1usize << a;
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let lq = b.alloc_n(a);
        let rq: Vec<Vec<Qubit>> = (0..lam).map(|_| b.alloc_n(w)).collect();
        let out = b.alloc_n(w);
        let words: Vec<Word> = regs
            .iter()
            .map(|&v| {
                let mut x = word(w);
                x[0] = v;
                x
            })
            .collect();
        FAULT.with(|c| c.set(fault));
        let h = low_hot(&mut b, &lq);
        let start = b.ops().len();
        mux(&mut b, &h.x, &rq, &out);
        let tof = b.ops()[start..]
            .iter()
            .filter(|o| matches!(o.kind, crate::circuit::OperationType::CCX))
            .count() as u64;
        low_unhot(&mut b, &lq, h);
        FAULT.with(|c| c.set(0));
        let mut sim = Sim::new(&b, seed);
        for (j, &q) in lq.iter().enumerate() {
            sim.set(q, low >> j & 1 == 1);
        }
        for (l, r) in rq.iter().enumerate() {
            for (k, &q) in r.iter().enumerate() {
                sim.set(q, regs[l] >> k & 1 == 1);
            }
        }
        sim.run(b.ops());
        let left = mux_leftover(&words, w)[0];
        let got = sim.read(&out) ^ left;
        // The registers and the index must come back unchanged; then clear them and the output.
        for (l, r) in rq.iter().enumerate() {
            assert_eq!(sim.read(r), regs[l], "register {l} changed");
        }
        assert_eq!(sim.read(&lq), low, "index changed");
        for &q in rq.iter().flatten().chain(&lq).chain(&out) {
            sim.set(q, false);
        }
        let clean =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sim.assert_clean())).is_ok();
        (got, clean, tof)
    }

    #[test]
    fn mux_selects_every_register_exactly() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for a in 1..=4usize {
            for w in [1usize, 2, 5, 6] {
                for _ in 0..6 {
                    let regs: Vec<u64> = (0..1 << a).map(|_| next() % (1 << w)).collect();
                    for low in 0..1u64 << a {
                        // The base block is loaded by the caller; here it is 0, so the result is
                        // the selected register for low >= 1 and 0 for low = 0.
                        let want = if low == 0 { 0 } else { regs[low as usize] };
                        for seed in 1..3 {
                            let (got, clean, tof) = run(a, w, &regs, low, 0, seed);
                            assert_eq!(got, want, "a {a} w {w} low {low}");
                            assert!(clean, "a {a} w {w} low {low}: dirty");
                            assert_eq!(tof as usize, mux_cost((1 << a) - 1, w), "cost");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mux_mutants_are_caught() {
        for fault in [40u8, 41] {
            let caught = (1..=3usize).any(|a| {
                let regs: Vec<u64> = (0..1u64 << a).map(|l| (l * 0x5B + 3) % 64).collect();
                (0..1u64 << a).any(|low| {
                    (1..4).any(|seed| {
                        let want = if low == 0 { 0 } else { regs[low as usize] };
                        let (got, clean, _) = run(a, 6, &regs, low, fault, seed);
                        got != want || !clean
                    })
                })
            });
            assert!(caught, "mux fault {fault} not caught");
        }
    }
}
