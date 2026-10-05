//! The lower bound on `C_step` of any sa-nested walk on a pinned
//! sos-sa spec, term by term, from the spec's own data. No circuit is built; release only.
//!
//! `floor_bound` prints, per spec and keep split (`SA_SPECS`, `SA_MU` as in `tests_combo`):
//!
//! - **Rotations.** Every square lane's copy applies the Householder reflection `V_u Z_0 V_u^+`
//!   of a dense `u` (no spec angle is 0), so its `Givens` sequence on the chain pairs `(j, j + 1)`
//!   holds an increasing and a decreasing chain sharing at most one rotation: at least `2 N - 3`
//!   per copy, `2 (2 N - 3)` per step at `2 (beta - 2)` each (the copies cannot share: copy 1 must
//!   apply `C H_a` for one leaf-independent `C`).
//! - **Angle delivery (union-span bound).** Over the lane domain every computational value is a
//!   function of the lane; CNOT, X, Swap, CZ, CCZ, S and measurement add no new function, and a
//!   `CCX` adds at most one dimension to the span `U` of every function that ever appears. Every
//!   `Givens` register must hold its rotation's angle word on every active lane, so between a
//!   copy's start and its Majorana, and again after it, `U` must cover the leaf-function space
//!   `D` spanned by the angle words. With `r` the affine GF(2) rank of the words (over the
//!   reachable leaves), `d0` the number of outer classes (one-body leaves plus square rows with a
//!   reachable leaf, the only leaf functions that can exist before the copy's inner read) and
//!   `L <= Q - 2 N - beta` the non-system live qubits at the Majorana:
//!   `T_copy >= (r + 1 - d0) + (r - L)`.
//! - **Construction charges** (not part of the theorem, which needs neither): four `SpinSwap`s
//!   `4 (N + 1)` and the reflections `u - 2`, `w - 2`.
//! - **Sign-variant robustness.** A circuit may deliver any of the `2^(N-1)` sign variants of a
//!   chain (`theta_j -> -theta_j`, `theta_(j+1) -> theta_(j+1) + pi`). Bit 0 of every word and
//!   `bit_k ^ bit_(k+1)` (`k + 1 <= beta - 2`) are invariant under a negation unless the angle's
//!   lowest set bit is exactly `k`; the rank of the invariant submatrix (columns for `k` in
//!   `k0..`, rows whose angles avoid those lowest bits) is a variant-proof `r`.
use super::{lane_map, Params, Tweaks};
use crate::lanemap::LaneMap;
use crate::spec::sa::SaSpec;
use crate::spec::EncodingSpec;

fn pinned(id: &str) -> Box<dyn EncodingSpec> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::spec::load(root, id).unwrap()
}

/// Affine GF(2) rank of bit rows (each row XORed with the first).
fn affine_rank(rows: &[Vec<u64>]) -> usize {
    let Some(r0) = rows.first() else { return 0 };
    let mut basis: Vec<(usize, Vec<u64>)> = Vec::new();
    for row in &rows[1..] {
        let mut r: Vec<u64> = row.iter().zip(r0).map(|(x, y)| x ^ y).collect();
        for (pc, bv) in &basis {
            if r[pc / 64] >> (pc % 64) & 1 == 1 {
                for (x, y) in r.iter_mut().zip(bv) {
                    *x ^= y;
                }
            }
        }
        let Some(pc) = (0..r.len() * 64).find(|&c| r[c / 64] >> (c % 64) & 1 == 1) else {
            continue;
        };
        for (_, bv) in &mut basis {
            if bv[pc / 64] >> (pc % 64) & 1 == 1 {
                for (x, y) in bv.iter_mut().zip(&r) {
                    *x ^= y;
                }
            }
        }
        basis.push((pc, r));
    }
    basis.len()
}

fn push_bit(row: &mut Vec<u64>, at: &mut usize, bit: bool) {
    if *at / 64 >= row.len() {
        row.push(0);
    }
    if bit {
        row[*at / 64] |= 1 << (*at % 64);
    }
    *at += 1;
}

/// The bound and its parts for one spec at one keep split.
#[allow(clippy::too_many_lines, clippy::needless_range_loop)]
fn report(id: &str, mu: Option<(u32, u32)>) {
    let boxed = pinned(id);
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let base = Params::for_spec(s);
    let p = Params {
        tw: Tweaks::parse("imchxgr"),
        outer: mu.map_or(base.outer, |m| (base.outer.0, m.0)),
        inner: mu.map_or(base.inner, |m| (base.inner.0, m.1)),
        ..base
    };
    let map = lane_map(s, p).unwrap();
    let (n, r, bb, c) = (s.n, s.r, s.b, s.c);
    let beta = s.beta as usize;
    // Reachable leaves: a one-body r with outer lanes; a square (r, b) with an outer item (r, c)
    // that has lanes and an inner count for b.
    let outer = map.outer.counts(n + r * c);
    let mut leaves: Vec<usize> = Vec::new();
    let mut rows_hit = vec![false; r];
    for rr in 0..r {
        for b in 0..bb {
            let hit = (0..c).any(|cc| {
                let item = n + rr * c + cc;
                outer[item] > 0 && map.inner[item].counts(bb + 1)[b] > 0
            });
            if hit {
                leaves.push(rr * bb + b);
                rows_hit[rr] = true;
            }
        }
    }
    let ob: Vec<usize> = (0..n).filter(|&k| outer[k] > 0).collect();
    leaves.extend(ob.iter().map(|&k| r * bb + k));
    let d0 = ob.len() + rows_hit.iter().filter(|&&h| h).count();
    let angle = |e: usize, j: usize| -> u64 {
        let net = if e < r * bb {
            &s.nets[e]
        } else {
            &s.e_nets[e - r * bb]
        };
        u64::from(net.rotations[j].2) & ((1 << beta) - 1)
    };
    let rot = n - 1;
    // Canonical words, all beta bits of every rotation.
    let canon: Vec<Vec<u64>> = leaves
        .iter()
        .map(|&e| {
            let (mut row, mut at) = (Vec::new(), 0);
            for j in 0..rot {
                let a = angle(e, j);
                for k in 0..beta {
                    push_bit(&mut row, &mut at, a >> k & 1 == 1);
                }
            }
            row
        })
        .collect();
    let r_can = affine_rank(&canon);
    let r_tail = affine_rank(
        &leaves
            .iter()
            .map(|&e| {
                let (mut row, mut at) = (Vec::new(), 0);
                for j in 1..rot {
                    let a = angle(e, j);
                    for k in 0..beta {
                        push_bit(&mut row, &mut at, a >> k & 1 == 1);
                    }
                }
                row
            })
            .collect::<Vec<_>>(),
    );
    let zero_angles = leaves
        .iter()
        .flat_map(|&e| (0..rot).map(move |j| (e, j)))
        .filter(|&(e, j)| angle(e, j) == 0)
        .count();
    // Negatable positions: every canonical angle below pi (top bit clear).
    let negatable: Vec<bool> = (0..rot)
        .map(|j| leaves.iter().all(|&e| angle(e, j) >> (beta - 1) == 0))
        .collect();
    let lsb = |a: u64| -> usize {
        if a == 0 {
            usize::MAX
        } else {
            a.trailing_zeros() as usize
        }
    };
    let mut robust = (0usize, 0usize, 0usize);
    for k0 in 1..beta - 2 {
        let keep: Vec<usize> = leaves
            .iter()
            .copied()
            .filter(|&e| {
                (0..rot).all(|j| {
                    !negatable[j] || {
                        let t = lsb(angle(e, j));
                        t < k0 || t > beta - 3
                    }
                })
            })
            .collect();
        let rows: Vec<Vec<u64>> = keep
            .iter()
            .map(|&e| {
                let (mut row, mut at) = (Vec::new(), 0);
                for j in 0..rot {
                    let a = angle(e, j);
                    if negatable[j] {
                        push_bit(&mut row, &mut at, a & 1 == 1);
                        for k in k0..=beta - 3 {
                            push_bit(&mut row, &mut at, (a >> k ^ a >> (k + 1)) & 1 == 1);
                        }
                    } else {
                        // Never negated alone: its low beta - 1 bits are invariant.
                        for k in 0..beta - 1 {
                            push_bit(&mut row, &mut at, a >> k & 1 == 1);
                        }
                    }
                }
                row
            })
            .collect();
        let rk = affine_rank(&rows);
        if rk > robust.0 {
            robust = (rk, k0, keep.len());
        }
    }
    let givens = 2 * (2 * n - 3);
    let g_cost = givens * 2 * (beta - 2);
    let swaps = 4 * (n + 1);
    let refl = (map.uniform_bits() - 2) as usize + (map.inner_width() - 2) as usize;
    let fixed_q = 2 * n + beta;
    let per_copy = |rk: usize, q: usize| -> usize {
        let l = q.saturating_sub(fixed_q);
        (rk + 1).saturating_sub(d0.min(l + 1)) + rk.saturating_sub(l)
    };
    let bound = |rk: usize, q: usize| g_cost + 2 * per_copy(rk, q);
    let min_q = |rk: usize, extra: usize, target: usize| -> Option<usize> {
        (fixed_q..4096).find(|&q| bound(rk, q) + extra < target)
    };
    println!(
        "FLOOR {id} mu {:?}: N {n} beta {beta} E {} reachable {} (one-body {}), zero angles {zero_angles}, d0 {d0}",
        (p.outer.1, p.inner.1),
        r * bb + n,
        leaves.len(),
        ob.len()
    );
    println!(
        "  ranks: canonical affine {r_can}, rotations 1.. {r_tail}; variant-proof {} (k0 {}, {} rows)",
        robust.0, robust.1, robust.2
    );
    println!(
        "  rotations >= {givens} Givens = {g_cost}; SpinSwap {swaps}; reflections {refl} (u {}, w {})",
        map.uniform_bits(),
        map.inner_width()
    );
    for q in [300usize, 317, 350, 400, 437, 479, 499, 600, 800, 1000, 1200] {
        println!(
            "  Q {q:>5}: C >= {:>6} (canonical) / {:>6} (+ SpinSwap, reflections); variant-proof {:>6} / {:>6}",
            bound(r_can, q),
            bound(r_can, q) + swaps + refl,
            bound(robust.0, q),
            bound(robust.0, q) + swaps + refl
        );
    }
    println!(
        "  C < 10,000 needs Q >= {:?} (canonical) / {:?} (+ SpinSwap, reflections); variant-proof {:?} / {:?}",
        min_q(r_can, 0, 10_000),
        min_q(r_can, swaps + refl, 10_000),
        min_q(robust.0, 0, 10_000),
        min_q(robust.0, swaps + refl, 10_000)
    );
}

#[test]
#[ignore = "pinned specs; run in release"]
fn floor_bound() {
    let mus: Vec<Option<(u32, u32)>> = std::env::var("SA_MU").map_or_else(
        |_| vec![Some((8, 8)), Some((9, 9))],
        |v| {
            let x: Vec<u32> = v.split(',').map(|y| y.parse().unwrap()).collect();
            vec![Some((x[0], x[1]))]
        },
    );
    for id in std::env::var("SA_SPECS")
        .unwrap_or_else(|_| "li-sa-est-v1,reiher-sa-est-v1".into())
        .split(',')
    {
        for &mu in &mus {
            report(id, mu);
        }
    }
}

/// Live qubits (the harness's liveness rule, `tests_combo::anatomy`) at every `Givens` of the
/// first copy and the peak: where a slot-0 gadget has headroom. `SA_SPECS`, `SA_TWEAKS`, `SA_MU`,
/// `SA_INNER_A` as in `tests_combo`.
#[test]
#[ignore = "pinned specs; run in release"]
fn givens_headroom() {
    use crate::circuit::{OperationType as K, NONE};
    let spec = std::env::var("SA_SPECS").unwrap_or_else(|_| "li-sa-est-v1".into());
    let tw = std::env::var("SA_TWEAKS").unwrap_or_else(|_| "imchxgrdky5zabCHVIXJ".into());
    let boxed = super::tests_combo::pinned(&spec);
    let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
    let p = super::tests_combo::params(s, &tw);
    let bt = super::tests_combo::build(s, p);
    let mut live: Vec<bool> = Vec::new();
    let (mut count, mut depth, mut best) = (0usize, 0usize, 0usize);
    let mut at_givens: Vec<(usize, usize)> = Vec::new();
    let touch = |q: u32, live: &mut Vec<bool>, count: &mut usize| {
        if q < bt.first {
            return;
        }
        let k = (q - bt.first) as usize;
        if k >= live.len() {
            live.resize(k + 1, false);
        }
        if !live[k] {
            live[k] = true;
            *count += 1;
        }
    };
    for (i, op) in bt.ops.iter().enumerate() {
        match op.kind {
            K::PushCondition => depth += 1,
            K::PopCondition => depth -= 1,
            _ => {}
        }
        match op.kind {
            K::Segment | K::Register | K::AppendToRegister | K::DebugPrint => {}
            K::R | K::Hmr if depth == 0 && op.c_condition == NONE => {
                if let Some(k) = op.q_target.checked_sub(bt.first) {
                    if live.get(k as usize).copied().unwrap_or(false) {
                        live[k as usize] = false;
                        count -= 1;
                    }
                }
            }
            K::Givens => {
                for &q in bt.registers.get(op.r_target as usize).into_iter().flatten() {
                    touch(q, &mut live, &mut count);
                }
                at_givens.push((i, count));
            }
            _ => op.qubits().for_each(|q| touch(q, &mut live, &mut count)),
        }
        best = best.max(count);
    }
    let off = bt.q_peak as usize - best;
    let n = at_givens.len();
    let show = |k: usize| at_givens[k].1 + off;
    println!(
        "HEADROOM {spec} {tw}: Q_peak {} over {} Givens; copy 1 V^dagger last 3 {:?}, V first 3 {:?}; copy 2 V^dagger last 3 {:?}, V first 3 {:?}",
        bt.q_peak,
        n,
        (n / 2 - n / 4 - 3..n / 2 - n / 4).map(show).collect::<Vec<_>>(),
        (n / 4..n / 4 + 3).map(show).collect::<Vec<_>>(),
        (3 * n / 4 - 3..3 * n / 4).map(show).collect::<Vec<_>>(),
        (3 * n / 4..3 * n / 4 + 3).map(show).collect::<Vec<_>>(),
    );
}

/// The **segmented** union-span bound for the chain-order class (each copy applies
/// `V^dagger` as rotations `N-2 .. 0` and `V` as `0 .. N-2`, one `Givens` per rotation and pass,
/// as every construction here and Low et al.'s does).
///
/// For any time interval `I` of a copy, every `CCX` in `I` adds at most one dimension to the span
/// of the live functions at the start of `I` plus everything made in `I`, and every word read by
/// a `Givens` in `I` lies in that span (on the active lanes). So `#CCX(I) >= u(I) - L(I)`, with
/// `u(I)` the affine GF(2) rank of the words of the rotations read in `I` over the reachable
/// leaves and `L(I)` the leaf-function dimension live at its start: at most `Q - 2 N - beta`, and
/// at most `d0` at the copy's start (TN1's outer-class argument). Summing over a partition of
/// the copy's `2 (N - 1)` Givens into consecutive segments (the first one starting at the copy's
/// start) and maximising by dynamic programming gives a per-copy bound; the step is twice it plus
/// the rotation floor `2 (2N - 3) 2 (beta - 2)`. With one segment per pass it is TN1.
#[test]
#[ignore = "pinned specs; run in release"]
#[allow(clippy::needless_range_loop, clippy::too_many_lines)]
fn segmented_floor() {
    let mu: Option<(u32, u32)> = std::env::var("SA_MU").ok().map(|v| {
        let x: Vec<u32> = v.split(',').map(|y| y.parse().unwrap()).collect();
        (x[0], x[1])
    });
    let qs: Vec<usize> = std::env::var("SA_QS")
        .unwrap_or_else(|_| "261,265,286,300,311,314,317,350,400,437,479,499,600,811".into())
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    for id in std::env::var("SA_SPECS")
        .unwrap_or_else(|_| "reiher-sa-est-v1,li-sa-est-v1".into())
        .split(',')
    {
        let boxed = pinned(id);
        let s: &SaSpec = boxed.as_any().downcast_ref().unwrap();
        let base = Params::for_spec(s);
        let p = Params {
            tw: Tweaks::parse("imchxgr"),
            outer: mu.map_or(base.outer, |m| (base.outer.0, m.0)),
            inner: mu.map_or(base.inner, |m| (base.inner.0, m.1)),
            ..base
        };
        let map = lane_map(s, p).unwrap();
        let (n, r, bb, c) = (s.n, s.r, s.b, s.c);
        let beta = s.beta as usize;
        let outer = map.outer.counts(n + r * c);
        let mut leaves: Vec<usize> = Vec::new();
        let mut rows_hit = vec![false; r];
        for rr in 0..r {
            for b in 0..bb {
                let hit = (0..c).any(|cc| {
                    let item = n + rr * c + cc;
                    outer[item] > 0 && map.inner[item].counts(bb + 1)[b] > 0
                });
                if hit {
                    leaves.push(rr * bb + b);
                    rows_hit[rr] = true;
                }
            }
        }
        let ob: Vec<usize> = (0..n).filter(|&k| outer[k] > 0).collect();
        leaves.extend(ob.iter().map(|&k| r * bb + k));
        let d0 = ob.len() + rows_hit.iter().filter(|&&h| h).count();
        let angle = |e: usize, j: usize| -> u64 {
            let net = if e < r * bb {
                &s.nets[e]
            } else {
                &s.e_nets[e - r * bb]
            };
            u64::from(net.rotations[j].2) & ((1 << beta) - 1)
        };
        let rot = n - 1;
        let words = leaves.len().div_ceil(64);
        // Affine columns: bit k of rotation j over the leaves, XORed with the first leaf's bit.
        let cols: Vec<Vec<Vec<u64>>> = (0..rot)
            .map(|j| {
                (0..beta)
                    .map(|k| {
                        let b0 = angle(leaves[0], j) >> k & 1;
                        let mut v = vec![0u64; words];
                        for (x, &e) in leaves.iter().enumerate() {
                            if (angle(e, j) >> k & 1) ^ b0 == 1 {
                                v[x / 64] |= 1 << (x % 64);
                            }
                        }
                        v
                    })
                    .collect()
            })
            .collect();
        let seq: Vec<usize> = (0..rot).rev().chain(0..rot).collect();
        let m = seq.len();
        // rank[j][i]: affine rank of the words of seq[j..i] (incremental basis per start j).
        let mut rank = vec![vec![0usize; m + 1]; m + 1];
        for j in 0..m {
            let mut basis: Vec<(usize, Vec<u64>)> = Vec::new();
            for i in j..m {
                for col in &cols[seq[i]] {
                    let mut v = col.clone();
                    for (pc, bv) in &basis {
                        if v[pc / 64] >> (pc % 64) & 1 == 1 {
                            v.iter_mut().zip(bv).for_each(|(x, y)| *x ^= y);
                        }
                    }
                    if let Some(pc) = (0..words * 64).find(|&c| v[c / 64] >> (c % 64) & 1 == 1) {
                        for (_, bv) in &mut basis {
                            if bv[pc / 64] >> (pc % 64) & 1 == 1 {
                                bv.iter_mut().zip(&v).for_each(|(x, y)| *x ^= y);
                            }
                        }
                        basis.push((pc, v));
                    }
                }
                rank[j][i + 1] = basis.len();
            }
        }
        let g_cost = 2 * (2 * n - 3) * 2 * (beta - 2);
        let swaps = 4 * (n + 1);
        let refl = (map.uniform_bits() - 2) as usize + (map.inner_width() - 2) as usize;
        println!(
            "SEGFLOOR {id} mu {:?}: N {n} beta {beta} reachable {} d0 {d0} whole-copy rank {}",
            (p.outer.1, p.inner.1),
            leaves.len(),
            rank[0][m]
        );
        for &q in &qs {
            let l = q.saturating_sub(2 * n + beta);
            let mut best = vec![0usize; m + 1];
            for i in 1..=m {
                best[i] = best[i - 1];
                for j in 0..i {
                    let off = if j == 0 { d0.min(l) } else { l };
                    best[i] = best[i].max(best[j] + rank[j][i].saturating_sub(off));
                }
            }
            let tn1 = (rank[0][m] + 1).saturating_sub(d0.min(l + 1)) + rank[0][m].saturating_sub(l);
            println!(
                "  Q {q:>5}: L {l:>4}: per copy >= {:>5} (TN1 {tn1:>5}); C >= {:>6} / {:>6} with SpinSwap and reflections",
                best[m],
                g_cost + 2 * best[m],
                g_cost + 2 * best[m] + swaps + refl
            );
        }
    }
}
