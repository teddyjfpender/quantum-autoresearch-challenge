//! The two kernels of the Gaussian tracker's final check (`GaussianLane::measure` in
//! `crate::sim::gaussian`), rescheduled without changing what the reference's verdict sees.
//!
//! **Equality up to zero signs.** Call two finite floats *equal up to zero sign* when they are
//! bit-identical or both zeros. If every input of `+`, `-`, `*` or `/` (by a non-zero divisor) is
//! replaced by a value equal to it up to zero sign, the result is equal up to zero sign: a zero
//! operand only ever contributes a zero term or a zero factor, and IEEE rounding is
//! sign-symmetric. `abs`, `hypot`, `==` and `total_cmp` on non-negative norms do not see zero
//! signs. Hence, as long as every value is finite:
//!
//! - a product with an exactly-zero factor (`w_r = 0` or `y_r = 0`) is a zero, and adding a zero
//!   to a running sum leaves it equal up to zero sign, so such terms may be skipped *in place*
//!   (the order of the remaining terms is kept);
//! - `d * w_r - y_r` with `w_r = 0` is `-y_r` up to zero sign, so it may be applied as a sign
//!   flip;
//! - the Pfaffian's update `tau_j row_i - tau_i row_j` is a zero when `tau` and `row` vanish at
//!   `i` (or at `j`), so such entries may be skipped.
//!
//! The values these kernels return are therefore equal up to zero sign to the reference's, and
//! the reference verdict (`off <= AD_TOL`, `nearest_w` of the overlap) is identical: `off` is a
//! maximum of absolute values, `nearest_w` compares norms of differences. Every kernel checks
//! finiteness (inputs, every reflection coefficient, every value read by the elimination, the
//! outputs); when anything is not finite it declines, and the caller uses a kernel that performs
//! the reference's operations exactly (`ad_residual_exact`) or the reference tracker itself.
#![allow(clippy::needless_range_loop)]
use crate::sim::gaussian::pfaffian::C64;
use crate::sim::gaussian::Tv;

/// The start value of `f64`'s `Sum` (`-0.0` on the pinned toolchain), read from the standard
/// library itself so the kernels fold exactly as `.sum::<f64>()` does.
#[inline]
#[must_use]
pub fn sum_start() -> f64 {
    std::iter::empty::<f64>().sum::<f64>()
}

fn all_finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// Columns run side by side.
const CB: usize = 16;

/// `max |Ad(Y) (U (+) U) e_b - e_b|` over every basis vector of both blocks (the reference's
/// `ad_residual`): `ad_residual_sparse` when every input is finite and it completes, otherwise
/// `ad_residual_exact`.
#[must_use]
pub fn ad_residual(u: Option<&[f64]>, ys: &[&Tv], n: usize) -> f64 {
    let finite = u.is_none_or(all_finite) && ys.iter().all(|y| all_finite(&y.v));
    if finite && ys.iter().all(|y| y.v.len() == n) {
        if let Some(v) = ad_residual_sparse(u, ys, n) {
            return v;
        }
    }
    ad_residual_exact(u, ys, n)
}

/// The reference's `ad_residual` operation for operation: every column's reflection chain runs
/// the reference's float operations in the reference's order (sequential fold from
/// `sum_start`, `2 * sum`, then `d * w - y`), `CB` independent columns side by side. The maximum
/// of non-NaN absolute values does not depend on order, and any NaN gives `INFINITY` in both.
#[must_use]
pub fn ad_residual_exact(u: Option<&[f64]>, ys: &[&Tv], n: usize) -> f64 {
    let start = sum_start();
    let mut worst = 0.0f64;
    let mut y: Vec<[f64; CB]> = vec![[0.0; CB]; n];
    for block in [false, true] {
        let flip = ys.iter().filter(|y| y.odd != block).count() % 2 == 1;
        let same: Vec<&[f64]> = ys
            .iter()
            .filter(|y| y.odd == block)
            .map(|y| y.v.as_slice())
            .collect();
        let sign = if flip { -1.0 } else { 1.0 };
        for j0 in (0..n).step_by(CB) {
            let cb = CB.min(n - j0);
            for (r, yr) in y.iter_mut().enumerate() {
                for c in 0..CB {
                    let j = j0 + c;
                    yr[c] = if c >= cb {
                        0.0
                    } else {
                        match u {
                            Some(u) => u[r * n + j],
                            None => f64::from(u8::from(r == j)),
                        }
                    };
                }
            }
            for w in same.iter().rev() {
                // The reference zips `w` with `y`: the shorter length counts.
                let len = w.len().min(n);
                let mut acc = [start; CB];
                for (wr, yr) in w[..len].iter().zip(&y) {
                    for c in 0..CB {
                        acc[c] += wr * yr[c];
                    }
                }
                let mut d = [0.0; CB];
                for c in 0..CB {
                    d[c] = 2.0 * acc[c];
                }
                for (wr, yr) in w[..len].iter().zip(y.iter_mut()) {
                    for c in 0..CB {
                        yr[c] = d[c] * wr - yr[c];
                    }
                }
            }
            for (r, yr) in y.iter().enumerate() {
                for (c, v) in yr.iter().enumerate().take(cb) {
                    let e = (sign * v - f64::from(u8::from(r == j0 + c))).abs();
                    if e.is_nan() {
                        return f64::INFINITY;
                    }
                    worst = worst.max(e);
                }
            }
        }
    }
    worst
}

/// A small bit set over `0..n` rows.
#[derive(Clone)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, r: usize) {
        self.0[r / 64] |= 1 << (r % 64);
    }
    fn get(&self, r: usize) -> bool {
        self.0[r / 64] >> (r % 64) & 1 == 1
    }
    fn meets(&self, o: &Self) -> bool {
        self.0.iter().zip(&o.0).any(|(a, b)| a & b != 0)
    }
    fn or(&mut self, o: &Self) {
        self.0.iter_mut().zip(&o.0).for_each(|(a, b)| *a |= b);
    }
    fn xor(&mut self, o: &Self) {
        self.0.iter_mut().zip(&o.0).for_each(|(a, b)| *a ^= b);
    }
    fn clear(&mut self) {
        self.0.fill(0);
    }
}

/// A reflection vector's non-zero entries (row order) and support.
struct Sparse {
    idx: Vec<u32>,
    val: Vec<f64>,
    bits: Bits,
}

/// `ad_residual` skipping exact zeros (module docs); `None` when a value is not finite. Inputs
/// must be finite.
///
/// Per group of `CB` columns it keeps `y = sigma * stored` with `sigma_r = (-1)^(g + flip_r)`,
/// the rows that may be non-zero (`supp`), and for each reflection `w` (support `S`):
/// - if `S` misses `supp`, every product is a zero, `d` is a zero, and `y` becomes `-y`: `g`
///   flips;
/// - otherwise, with `ws_r = sigma_r w_r` on `S`: `acc = sum_{r in S} ws_r stored_r` (in row
///   order; `w_r y_r = sigma_r (ws_r stored_r)` exactly), `stored_r = d ws_r - stored_r` on `S`
///   (`= sigma_r (d w_r - y_r)` exactly), and every row outside `S` flips: `g` flips and
///   `flip_r` flips on `S`, which keeps `sigma_r` on `S`.
#[must_use]
pub fn ad_residual_sparse(u: Option<&[f64]>, ys: &[&Tv], n: usize) -> Option<f64> {
    let start = sum_start();
    let mut worst = 0.0f64;
    // Columns grouped by parity: the two spin sectors of the SA circuits, so a group's rows stay
    // within one sector and reflections in the other sector are skipped whole.
    let cols: Vec<usize> = (0..n).step_by(2).chain((1..n).step_by(2)).collect();
    let col_bits: Vec<Bits> = (0..n)
        .map(|j| {
            let mut b = Bits::new(n);
            for r in 0..n {
                let v = u.map_or(f64::from(u8::from(r == j)), |u| u[r * n + j]);
                if v != 0.0 {
                    b.set(r);
                }
            }
            b
        })
        .collect();
    let mut stored: Vec<[f64; CB]> = vec![[0.0; CB]; n];
    let mut supp = Bits::new(n);
    let mut flip_r = Bits::new(n);
    let mut ws: Vec<f64> = Vec::with_capacity(n);
    for block in [false, true] {
        let flip = ys.iter().filter(|y| y.odd != block).count() % 2 == 1;
        let sign = if flip { -1.0 } else { 1.0 };
        // The reflections in the reference's application order (`same.iter().rev()`).
        let refl: Vec<Sparse> = ys
            .iter()
            .rev()
            .filter(|y| y.odd == block)
            .map(|y| {
                let mut s = Sparse {
                    idx: Vec::new(),
                    val: Vec::new(),
                    bits: Bits::new(n),
                };
                for (r, &v) in y.v.iter().enumerate() {
                    if v != 0.0 {
                        s.idx.push(r as u32);
                        s.val.push(v);
                        s.bits.set(r);
                    }
                }
                s
            })
            .collect();
        for group in cols.chunks(CB) {
            let cb = group.len();
            supp.clear();
            flip_r.clear();
            let mut g = false;
            for (r, sr) in stored.iter_mut().enumerate() {
                for c in 0..CB {
                    sr[c] = if c < cb {
                        let j = group[c];
                        u.map_or(f64::from(u8::from(r == j)), |u| u[r * n + j])
                    } else {
                        0.0
                    };
                }
            }
            for &j in group {
                supp.or(&col_bits[j]);
            }
            for w in &refl {
                if !w.bits.meets(&supp) {
                    g = !g;
                    continue;
                }
                ws.clear();
                ws.extend(w.idx.iter().zip(&w.val).map(|(&r, &v)| {
                    if g ^ flip_r.get(r as usize) {
                        -v
                    } else {
                        v
                    }
                }));
                if !reflect(&mut stored, &w.idx, &ws, start) {
                    return None;
                }
                g = !g;
                flip_r.xor(&w.bits);
                supp.or(&w.bits);
            }
            for (r, sr) in stored.iter().enumerate() {
                let neg = g ^ flip_r.get(r);
                for c in 0..cb {
                    let v = if neg { -sr[c] } else { sr[c] };
                    if !v.is_finite() {
                        return None;
                    }
                    let e = (sign * v - f64::from(u8::from(r == group[c]))).abs();
                    worst = worst.max(e);
                }
            }
        }
    }
    Some(worst)
}

/// One reflection of a column group on the rows `idx` (`ws` = signed `w` there): `acc` folds
/// `ws_r * stored_r` in row order from `start`, `d = 2 acc`, then `stored_r = d ws_r -
/// stored_r`. `false` if some `d` is not finite.
fn reflect(stored: &mut [[f64; CB]], idx: &[u32], ws: &[f64], start: f64) -> bool {
    let mut acc = [start; CB];
    for (&r, &x) in idx.iter().zip(ws) {
        let sr = &stored[r as usize];
        for c in 0..CB {
            acc[c] += x * sr[c];
        }
    }
    let mut d = [0.0; CB];
    for c in 0..CB {
        d[c] = 2.0 * acc[c];
    }
    if !d.iter().all(|x| x.is_finite()) {
        return false;
    }
    for (&r, &x) in idx.iter().zip(ws) {
        let sr = &mut stored[r as usize];
        for c in 0..CB {
            sr[c] = d[c] * x - sr[c];
        }
    }
    true
}

fn cmul(a: C64, o: C64) -> C64 {
    C64 {
        re: a.re * o.re - a.im * o.im,
        im: a.re * o.im + a.im * o.re,
    }
}

fn cdiv(a: C64, o: C64) -> C64 {
    let d = o.re * o.re + o.im * o.im;
    C64 {
        re: (a.re * o.re + a.im * o.im) / d,
        im: (a.im * o.re - a.re * o.im) / d,
    }
}

fn cnorm(a: C64) -> f64 {
    a.re.hypot(a.im)
}

/// `<vac| gamma(y_1) ... gamma(y_m) |vac>` (the reference's `pfaffian::vacuum_expectation`),
/// equal to it up to zero signs (module docs); `None` when an input or a value the elimination
/// reads is not finite (the caller then uses the reference).
#[must_use]
pub fn vacuum_expectation(ys: &[&Tv]) -> Option<C64> {
    let m = ys.len();
    if m % 2 == 1 {
        return Some(C64 { re: 0.0, im: 0.0 });
    }
    if m == 0 {
        return Some(C64 { re: 1.0, im: 0.0 });
    }
    let n = ys[0].v.len();
    if ys.iter().any(|y| y.v.len() != n || !all_finite(&y.v)) {
        return None;
    }
    // Upper triangle of the contraction matrix, row-major m x m, split real / imaginary. The
    // buffers are reused per thread: `gram` writes every upper entry and nothing reads the
    // lower triangle, so stale contents are never observed.
    thread_local! {
        static BUF: std::cell::RefCell<(Vec<f64>, Vec<f64>)> =
            const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
    }
    let (mut re, mut im) = BUF.with(|b| std::mem::take(&mut *b.borrow_mut()));
    re.resize(m * m, 0.0);
    im.resize(m * m, 0.0);
    let panel = elim_panel();
    let r = if real_elim() && panel > 0 {
        // The coefficients in `re` (`pfreal`); the parities in a copy (a swap moves them).
        let t = super::prof::start();
        gram_coef(ys, n, m, &mut re);
        super::prof::stop(super::prof::GRAM, t);
        let t = super::prof::start();
        let r = if (1..m).all(|j| re[j] != 0.0) || connected(&re, m) {
            // One component of the non-zero pattern (row 0 with no zero entry joins every
            // index): the dense elimination. (It is the reference's elimination on any input;
            // the component search below only saves work on split patterns.)
            let mut par: Vec<bool> = ys.iter().map(|y| y.odd).collect();
            super::pfreal::pfaffian_real(&mut re, &mut par, m, panel)
        } else {
            // The component search, on the complex parts (`gram`'s placement up to zero signs).
            for i in 0..m {
                for j in i + 1..m {
                    let c = re[i * m + j];
                    let (a, b) = if ys[i].odd == ys[j].odd {
                        (c, 0.0)
                    } else {
                        (0.0, c)
                    };
                    re[i * m + j] = a;
                    im[i * m + j] = b;
                }
            }
            pfaffian_components(&mut re, &mut im, m, panel)
        };
        super::prof::stop(super::prof::ELIM, t);
        r
    } else {
        let t = super::prof::start();
        gram(ys, n, m, &mut re, &mut im);
        super::prof::stop(super::prof::GRAM, t);
        let t = super::prof::start();
        let r = pfaffian_components(&mut re, &mut im, m, panel);
        super::prof::stop(super::prof::ELIM, t);
        r
    };
    BUF.with(|b| *b.borrow_mut() = (re, im));
    r
}

/// Whether the non-zero pattern of the upper triangle `c` (row-major `m x m`) joins every
/// index into one component.
fn connected(c: &[f64], m: usize) -> bool {
    let mut parent: Vec<usize> = (0..m).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut comps = m;
    for i in 0..m {
        for j in i + 1..m {
            if c[i * m + j] != 0.0 {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                    comps -= 1;
                    if comps == 1 {
                        return true;
                    }
                }
            }
        }
    }
    comps <= 1
}

/// Whether the dense overlap runs on real coefficients (`pfreal`; default) or on complex
/// entries (`pfblock`); `FEMOCO_FASTSIM_REAL=0` selects the latter (tooling for A/B runs and
/// tests, the result is the same either way).
#[must_use]
pub fn real_elim() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("FEMOCO_FASTSIM_REAL").map_or(true, |v| v != "0"))
}

/// `gram`, for the timing test.
#[cfg(test)]
pub fn gram_for_tests(ys: &[&Tv], n: usize, m: usize, re: &mut [f64], im: &mut [f64]) {
    gram(ys, n, m, re, im);
}

/// The upper triangle of the contraction matrix (the reference's `contraction` for `i < j`):
/// `d = sum_r ys[i].v[r] ys[j].v[r]`, folded in `r` from `sum_start` (terms with a zero factor
/// skipped, module docs), then placed by parity. Inputs must be finite.
#[inline(never)]
fn gram(ys: &[&Tv], n: usize, m: usize, re: &mut [f64], im: &mut [f64]) {
    let (ids, k, dots) = gram_dots(ys, n, m);
    // The reference's placement: (even, odd) i d, (odd, even) -i d, else d. As products with
    // 1, -1 and 0 (exact for finite d, up to the sign of a zero).
    let (mut even, mut oddv, mut neg_even) = (vec![0.0; m], vec![0.0; m], vec![0.0; m]);
    for (j, y) in ys.iter().enumerate() {
        if y.odd {
            oddv[j] = 1.0;
        } else {
            even[j] = 1.0;
            neg_even[j] = -1.0;
        }
    }
    let mut drow = vec![0.0f64; m];
    for i in 0..m {
        let row = &dots[ids[i] * k..(ids[i] + 1) * k];
        for j in i + 1..m {
            drow[j] = row[ids[j]];
        }
        let (cr, ci) = if ys[i].odd {
            (&oddv, &neg_even)
        } else {
            (&even, &oddv)
        };
        let (rr, ri) = (&mut re[i * m..(i + 1) * m], &mut im[i * m..(i + 1) * m]);
        for j in i + 1..m {
            rr[j] = drow[j] * cr[j];
            ri[j] = drow[j] * ci[j];
        }
    }
}

/// The distinct vectors' dot products (`dots[a k + b]`, both orientations) and each factor's
/// distinct vector (`ids`), for `gram` and `gram_coef`.
fn gram_dots(ys: &[&Tv], n: usize, m: usize) -> (Vec<usize>, usize, Vec<f64>) {
    // Distinct vectors (bitwise). A pair's two factors carry the same floats (one per block),
    // and a mode untouched between two hand-offs repeats a vector: their dot products with any
    // other vector are the same float operations, so each is computed once.
    let mut ids = vec![0usize; m];
    let mut uq: Vec<&Tv> = Vec::with_capacity(m);
    let mut seen: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
    for (i, y) in ys.iter().enumerate() {
        // A pair's second factor follows its first: compare with the previous one first.
        if i > 0
            && ys[i - 1]
                .v
                .iter()
                .zip(&y.v)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        {
            ids[i] = ids[i - 1];
            continue;
        }
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for x in &y.v {
            h = (h ^ x.to_bits()).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let same = |c: &usize| {
            uq[*c]
                .v
                .iter()
                .zip(&y.v)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        };
        let found = seen
            .get(&h)
            .and_then(|cs| cs.iter().copied().find(|c| same(c)));
        ids[i] = match found {
            Some(c) => c,
            None => {
                seen.entry(h).or_default().push(uq.len());
                uq.push(y);
                uq.len() - 1
            }
        };
    }
    let k = uq.len();
    let start = sum_start();
    let mut dots = vec![0.0f64; k * k];
    for (c, y) in uq.iter().enumerate() {
        let mut acc = start;
        for &x in &y.v {
            if x != 0.0 {
                acc += x * x;
            }
        }
        dots[c * k + c] = acc;
    }
    #[cfg(target_arch = "aarch64")]
    gram_dense(&uq, n, k, &mut dots);
    #[cfg(not(target_arch = "aarch64"))]
    gram_core(&uq, n, k, &mut |a, b, d| dots[a * k + b] = d);
    // Both orientations, so a row of `dots` serves every j.
    for a in 0..k {
        for b in a + 1..k {
            dots[b * k + a] = dots[a * k + b];
        }
    }
    (ids, k, dots)
}

/// The reference's contraction matrix as coefficients (`pfreal` module docs): `c[i m + j]`,
/// `i < j`, is the part of entry `(i, j)` that the reference's `contraction` sets to `d` or
/// `-d` (the other part is the literal `0.0`): `-d` exactly for (odd, even), else `d`.
#[inline(never)]
fn gram_coef(ys: &[&Tv], n: usize, m: usize, c: &mut [f64]) {
    let (ids, k, dots) = gram_dots(ys, n, m);
    for i in 0..m {
        let row = &dots[ids[i] * k..(ids[i] + 1) * k];
        let out = &mut c[i * m..(i + 1) * m];
        if ys[i].odd {
            for j in i + 1..m {
                let d = row[ids[j]];
                out[j] = if ys[j].odd { d } else { -d };
            }
        } else {
            for j in i + 1..m {
                out[j] = row[ids[j]];
            }
        }
    }
}

/// `dots[a k + b]`, `a < b < k`: `sum_r ys[a].v[r] ys[b].v[r]` folded in `r` from `sum_start`
/// with every term (the reference's `contraction` exactly), four rows by eight columns at a
/// time on two-lane registers. Inputs must be finite.
#[cfg(target_arch = "aarch64")]
fn gram_dense(ys: &[&Tv], n: usize, k: usize, dots: &mut [f64]) {
    use core::arch::aarch64::{
        vaddq_f64, vdupq_n_f64, vld1q_dup_f64, vld1q_f64, vmulq_f64, vst1q_f64,
    };
    let kp = k.div_ceil(8) * 8;
    let mut yt = vec![0.0f64; n * kp];
    for (j, y) in ys.iter().enumerate() {
        for (r, &v) in y.v.iter().enumerate().take(n) {
            yt[r * kp + j] = v;
        }
    }
    let zero = vec![0.0f64; n];
    let start = sum_start();
    let mut out = [0.0f64; 32];
    let mut i0 = 0;
    while i0 < k {
        let xs: [&[f64]; 4] =
            std::array::from_fn(|t| ys.get(i0 + t).map_or(zero.as_slice(), |y| &y.v[..n]));
        let mut j0 = (i0 + 1) / 8 * 8;
        while j0 < k {
            // SAFETY: every load is inside `yt` (`r < n`, `j0 + 8 <= kp`) or a row of length
            // `n`; `out` holds 32 values.
            unsafe {
                let mut acc = [[vdupq_n_f64(start); 4]; 4];
                for r in 0..n {
                    let yp = yt.as_ptr().add(r * kp + j0);
                    let y = [
                        vld1q_f64(yp),
                        vld1q_f64(yp.add(2)),
                        vld1q_f64(yp.add(4)),
                        vld1q_f64(yp.add(6)),
                    ];
                    for t in 0..4 {
                        let x = vld1q_dup_f64(xs[t].as_ptr().add(r));
                        for q in 0..4 {
                            acc[t][q] = vaddq_f64(acc[t][q], vmulq_f64(x, y[q]));
                        }
                    }
                }
                for t in 0..4 {
                    for q in 0..4 {
                        vst1q_f64(out.as_mut_ptr().add(8 * t + 2 * q), acc[t][q]);
                    }
                }
            }
            for t in 0..4 {
                let i = i0 + t;
                for c in 0..8 {
                    let j = j0 + c;
                    if i < j && j < k {
                        dots[i * k + j] = out[8 * t + c];
                    }
                }
            }
            j0 += 8;
        }
        i0 += 4;
    }
}

/// `put(i, j, d)` for every `i < j < m`, `d = sum_r ys[i].v[r] ys[j].v[r]` folded in `r` from
/// `sum_start` (terms with a zero factor skipped, module docs). Inputs must be finite.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn gram_core(ys: &[&Tv], n: usize, m: usize, put: &mut dyn FnMut(usize, usize, f64)) {
    // yt[r * mp + j] = ys[j].v[r], columns padded with zeros to a multiple of 8.
    let mp = m.div_ceil(8) * 8 + 8;
    let mut yt = vec![0.0f64; n * mp];
    for (j, y) in ys.iter().enumerate() {
        for (r, &v) in y.v.iter().enumerate() {
            yt[r * mp + j] = v;
        }
    }
    let start = sum_start();
    // Tiles of 4 rows x 8 columns; row i keeps only its columns j > i.
    let mut i0 = 0;
    while i0 + 4 <= m {
        let xs: [&[f64]; 4] = std::array::from_fn(|t| ys[i0 + t].v.as_slice());
        let live: Vec<usize> = (0..n).filter(|&r| xs.iter().any(|x| x[r] != 0.0)).collect();
        let mut j0 = i0 + 1;
        while j0 < m {
            let acc = tile(&xs, &live, &yt[j0..], mp, start);
            for (t, row) in acc.iter().enumerate() {
                let i = i0 + t;
                for (c, &d) in row.iter().enumerate() {
                    let j = j0 + c;
                    if j > i && j < m {
                        put(i, j, d);
                    }
                }
            }
            j0 += 8;
        }
        i0 += 4;
    }
    let mut acc = vec![0.0f64; mp];
    for i in i0..m.saturating_sub(1) {
        let lo = i + 1;
        acc[lo..m].fill(start);
        for (r, &x) in ys[i].v.iter().enumerate() {
            if x == 0.0 {
                continue;
            }
            let row = &yt[r * mp + lo..r * mp + m];
            for (a, &yv) in acc[lo..m].iter_mut().zip(row) {
                *a += x * yv;
            }
        }
        for j in lo..m {
            put(i, j, acc[j]);
        }
    }
}

/// `acc[t][c] = fold_r (xs[t][r] * yt[r * mp + c])` over the rows `live`, from `start`.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn tile(xs: &[&[f64]; 4], live: &[usize], yt: &[f64], mp: usize, start: f64) -> [[f64; 8]; 4] {
    let mut a = [[start; 8]; 4];
    for &r in live {
        let y: &[f64; 8] = yt[r * mp..r * mp + 8].try_into().unwrap_or(&[0.0; 8]);
        for t in 0..4 {
            let x = xs[t][r];
            for c in 0..8 {
                a[t][c] += x * y[c];
            }
        }
    }
    a
}

/// `|z|` as the reference's `C64::norm` (`hypot`) computes it. `hypot(x, +-0) = |x|` and
/// `hypot(+-0, y) = |y|` (C11 Annex F; the platform's documented special values), so the call
/// is made only when both parts are non-zero.
#[inline]
fn norm2(re: f64, im: f64) -> f64 {
    if im == 0.0 {
        re.abs()
    } else if re == 0.0 {
        im.abs()
    } else {
        re.hypot(im)
    }
}

/// The reference's pivoted elimination, run per connected component of the contraction
/// matrix's non-zero pattern (`gram`'s upper triangle, logical index order).
///
/// **Why the result is the reference's up to zero signs.** An entry between two components
/// is an exact zero initially (`gram` adds only zero products), and stays one: the update of
/// `(i, j)` at step `k` is `tau_j row_i - tau_i row_j`, where `tau` is non-zero only in the
/// component of `k` and `row` only in that of the pivot, which is the same component (its
/// entry in row `k` is non-zero). So each step reads and writes one component.
///
/// - **Logical order.** The reference swaps logical indices `k + 1` and `piv`; that is a pure
///   permutation of its full matrix, kept here as maps between logical indices and
///   (component, position). No data moves.
/// - **Storage.** Each component stores, for positions `a < b`, the entry (element at `a`,
///   element at `b`); the reference holds both orientations, `A(x, y)` and `A(y, x)`, and they
///   are negatives up to zero sign (initially exactly; every update of one is the negated
///   update of the other, and IEEE subtraction is sign-symmetric). Reading `A(x, y)` with `x`
///   stored after `y` therefore negates.
/// - **Compaction.** After a step, the two eliminated elements are moved to the front of their
///   component, so the elements still to be read form a contiguous suffix. Moving an element
///   past another flips their stored orientation (a negation).
/// - **Pivot.** The reference takes the last maximum of `|A(k, x)|` over `x > k` in logical
///   order. Entries outside the component are zeros, so the maximum lies in the component
///   unless the component's row is all zero, in which case the reference's head is a zero and
///   it returns 0, as here.
/// - Everything else (`tau`, `row`, the update formula, `pf`) is the reference's arithmetic on
///   values equal up to zero sign (module docs); rows whose `tau` and `row` are both zero
///   receive zero updates and are skipped.
///
/// `None` when a value read is not finite.
#[inline(never)]
pub(crate) fn pfaffian_components(
    up_re: &mut Vec<f64>,
    up_im: &mut Vec<f64>,
    m: usize,
    panel: usize,
) -> Option<C64> {
    // Row 0 with no zero entry joins every index: one component, no search needed.
    if panel > 0 && (1..m).all(|j| up_re[j] != 0.0 || up_im[j] != 0.0) {
        return super::pfblock::pfaffian_dense(up_re, up_im, m, panel);
    }
    // Components of the non-zero pattern (union-find).
    let mut parent: Vec<usize> = (0..m).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for i in 0..m {
        for j in i + 1..m {
            if up_re[i * m + j] != 0.0 || up_im[i * m + j] != 0.0 {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut comps: Vec<Comp> = Vec::new();
    let mut root_comp = vec![usize::MAX; m];
    // place[x] = (component, position) of logical index x.
    let mut place = vec![(0usize, 0usize); m];
    for x in 0..m {
        let r = find(&mut parent, x);
        if root_comp[r] == usize::MAX {
            root_comp[r] = comps.len();
            comps.push(Comp::default());
        }
        let c = root_comp[r];
        place[x] = (c, comps[c].elem.len());
        comps[c].elem.push(x);
    }
    if comps.len() == 1 && panel > 0 {
        // One component (the dense case): the delayed-update form of the same elimination.
        return super::pfblock::pfaffian_dense(up_re, up_im, m, panel);
    }
    if comps.len() == 1 {
        // One component in logical order: its storage is the contraction matrix itself.
        comps[0].re = std::mem::take(up_re);
        comps[0].im = std::mem::take(up_im);
        let r = eliminate(&mut comps, &mut place, m);
        *up_re = std::mem::take(&mut comps[0].re);
        *up_im = std::mem::take(&mut comps[0].im);
        return r;
    }
    for c in &mut comps {
        let s = c.elem.len();
        c.re = vec![0.0; s * s];
        c.im = vec![0.0; s * s];
        for a in 0..s {
            for b in a + 1..s {
                // Members are in increasing logical order: (a, b) is the reference's upper entry.
                let (x, y) = (c.elem[a], c.elem[b]);
                c.re[a * s + b] = up_re[x * m + y];
                c.im[a * s + b] = up_im[x * m + y];
            }
        }
    }
    eliminate(&mut comps, &mut place, m)
}

/// Steps per panel of the dense elimination (`pfblock`); 0 selects the per-component
/// elimination below for dense inputs too (`FEMOCO_FASTSIM_PANEL`; tooling for A/B runs and
/// tests, the result is the same either way).
#[must_use]
pub fn elim_panel() -> usize {
    static V: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("FEMOCO_FASTSIM_PANEL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(super::pfblock::PANEL)
    })
}

/// The elimination loop of `pfaffian_components` over prepared components.
fn eliminate(comps: &mut [Comp], place: &mut [(usize, usize)], m: usize) -> Option<C64> {
    let mut pf = C64 { re: 1.0, im: 0.0 };
    let mut tau_r: Vec<f64> = Vec::new();
    let mut tau_i: Vec<f64> = Vec::new();
    let mut row_r: Vec<f64> = Vec::new();
    let mut row_i: Vec<f64> = Vec::new();
    for k in (0..m).step_by(2) {
        let (c, pk) = place[k];
        let comp = &mut comps[c];
        let s = comp.elem.len();
        // Pivot: the last maximum of |A(k, x)| over the component's elements x > k.
        let mut piv = usize::MAX;
        let mut best = 0.0f64;
        for pos in comp.start..s {
            let x = comp.elem[pos];
            if x <= k {
                continue;
            }
            let (vr, vi) = comp.get(pk, pos);
            let v = norm2(vr, vi);
            if !v.is_finite() {
                return None;
            }
            if v > best || (v == best && v > 0.0 && x > piv) {
                best = v;
                piv = x;
            }
        }
        if best == 0.0 {
            // The reference's head is a zero (its whole row k vanishes).
            return Some(C64 { re: 0.0, im: 0.0 });
        }
        if piv != k + 1 {
            // Logical swap of k + 1 and piv (piv is in this component; k + 1 may not be).
            let a = place[k + 1];
            let b = place[piv];
            place.swap(k + 1, piv);
            comps[a.0].elem[a.1] = piv;
            comps[b.0].elem[b.1] = k + 1;
            pf = C64 {
                re: -pf.re,
                im: -pf.im,
            };
        }
        let comp = &mut comps[c];
        let p1 = place[k + 1].1;
        let (hr, hi) = comp.get(pk, p1);
        let head = C64 { re: hr, im: hi };
        if cnorm(head) == 0.0 {
            return Some(C64 { re: 0.0, im: 0.0 });
        }
        pf = cmul(pf, head);
        // tau and row for the remaining elements, by element; then compaction.
        let s = comp.elem.len();
        tau_r.clear();
        tau_i.clear();
        row_r.clear();
        row_i.clear();
        let mut by_elem: Vec<(usize, f64, f64, f64, f64)> = Vec::with_capacity(s);
        for pos in comp.start..s {
            if pos == pk || pos == p1 {
                continue;
            }
            let (ar, ai) = comp.get(pk, pos);
            let t = cdiv(C64 { re: ar, im: ai }, head);
            let (r0, r1) = comp.get(p1, pos);
            if !(t.re.is_finite() && t.im.is_finite() && r0.is_finite() && r1.is_finite()) {
                return None;
            }
            by_elem.push((comp.elem[pos], t.re, t.im, r0, r1));
        }
        // Move k and k + 1 to the front of the remaining region.
        for e in [k, k + 1] {
            let pos = place[e].1;
            let front = comp.start;
            if pos != front {
                let other = comp.elem[front];
                comp.swap_positions(front, pos);
                place[other] = (c, pos);
                place[e] = (c, front);
            }
            comp.start += 1;
        }
        // Remaining elements now occupy start..s; lay tau/row out by position.
        let start = comp.start;
        let rest = s - start;
        tau_r.resize(rest, 0.0);
        tau_i.resize(rest, 0.0);
        row_r.resize(rest, 0.0);
        row_i.resize(rest, 0.0);
        for &(x, tr, ti, r0, r1) in &by_elem {
            let at = place[x].1 - start;
            tau_r[at] = tr;
            tau_i[at] = ti;
            row_r[at] = r0;
            row_i[at] = r1;
        }
        comp.update(start, (&tau_r, &tau_i, &row_r, &row_i));
    }
    (pf.re.is_finite() && pf.im.is_finite()).then_some(pf)
}

/// One component of the elimination (`pfaffian_components`).
#[derive(Default)]
struct Comp {
    /// Logical index of the element at each position.
    elem: Vec<usize>,
    /// Positions `start..` hold the elements not yet eliminated.
    start: usize,
    /// Entry (element at `a`, element at `b`) for `a < b`, row-major `s x s`.
    re: Vec<f64>,
    im: Vec<f64>,
}

impl Comp {
    /// `A(element at a, element at b)` (up to zero sign), `a != b`.
    #[inline]
    fn get(&self, a: usize, b: usize) -> (f64, f64) {
        let s = self.elem.len();
        if a < b {
            (self.re[a * s + b], self.im[a * s + b])
        } else {
            (-self.re[b * s + a], -self.im[b * s + a])
        }
    }

    /// Exchanges the elements at positions `x < y` among the positions `start..` (the only
    /// ones read later), keeping every stored entry's meaning: an entry whose two elements
    /// change order is negated.
    #[inline(never)]
    fn swap_positions(&mut self, x: usize, y: usize) {
        let (x, y) = (x.min(y), x.max(y));
        let s = self.elem.len();
        self.elem.swap(x, y);
        for a in [&mut self.re, &mut self.im] {
            for j in y + 1..s {
                a.swap(x * s + j, y * s + j);
            }
            for i in self.start..x {
                a.swap(i * s + x, i * s + y);
            }
            for j in x + 1..y {
                let (axj, ajy) = (a[x * s + j], a[j * s + y]);
                a[x * s + j] = -ajy;
                a[j * s + y] = -axj;
            }
            a[x * s + y] = -a[x * s + y];
        }
    }

    /// The rank-2 update of every entry `(a, b)`, `start <= a < b`: the reference's
    /// `tau_b row_a - tau_a row_b` (element-wise as `upd`). Rows are taken two at a time to
    /// share the loads of `tau_b`, `row_b`; each entry receives exactly the reference's
    /// operations.
    #[inline(never)]
    fn update(&mut self, start: usize, (tr, ti, rr, ri): (&[f64], &[f64], &[f64], &[f64])) {
        let s = self.elem.len();
        let rest = s - start;
        if rest < 2 {
            return;
        }
        let mut a = 0;
        while a + 1 < rest {
            let ca = (tr[a], ti[a], rr[a], ri[a]);
            let cn = (tr[a + 1], ti[a + 1], rr[a + 1], ri[a + 1]);
            let ra = (start + a) * s + start;
            let rb = ra + s;
            // (a, a + 1) alone.
            {
                let (vr, vi) = (&mut self.re[ra + a + 1], &mut self.im[ra + a + 1]);
                upd(vr, vi, ca, (tr[a + 1], ti[a + 1]), (rr[a + 1], ri[a + 1]));
            }
            let lo = a + 2;
            if lo < rest {
                let len = rest - lo;
                let (re_a, re_b) = self.re.split_at_mut(rb);
                let (im_a, im_b) = self.im.split_at_mut(rb);
                let rows = [
                    &mut re_a[ra + lo..ra + rest],
                    &mut im_a[ra + lo..ra + rest],
                    &mut re_b[lo..rest],
                    &mut im_b[lo..rest],
                ];
                let cols = [&tr[lo..rest], &ti[lo..rest], &rr[lo..rest], &ri[lo..rest]];
                pair_update(rows, cols, len, ca, cn);
            }
            a += 2;
        }
    }
}

/// One element's update, `d = tau_j row_i - tau_i row_j` (`C64::mul`, then `C64::sub`), added
/// to `(vr, vi)`.
#[inline(always)]
fn upd(
    vr: &mut f64,
    vi: &mut f64,
    (tir, tii, rir, rii): (f64, f64, f64, f64),
    tj: (f64, f64),
    rj: (f64, f64),
) {
    let ar = tj.0 * rir - tj.1 * rii;
    let ai = tj.0 * rii + tj.1 * rir;
    let br = tir * rj.0 - tii * rj.1;
    let bi = tir * rj.1 + tii * rj.0;
    *vr += ar - br;
    *vi += ai - bi;
}

/// Rows `i` (`rows[0]`, `rows[1]`: real, imaginary) and `i + 1` (`rows[2]`, `rows[3]`) over
/// the same columns `j` (`cols`: `tau_j` real, imaginary, `row_j` real, imaginary).
#[allow(clippy::many_single_char_names)]
#[inline(never)]
fn pair_update(
    rows: [&mut [f64]; 4],
    cols: [&[f64]; 4],
    len: usize,
    ci: (f64, f64, f64, f64),
    cn: (f64, f64, f64, f64),
) {
    let [ra, ia, rb, ib] = rows;
    let [tjr, tji, rjr, rji] = cols;
    let (ra, ia, rb, ib) = (
        &mut ra[..len],
        &mut ia[..len],
        &mut rb[..len],
        &mut ib[..len],
    );
    let (tjr, tji, rjr, rji) = (&tjr[..len], &tji[..len], &rjr[..len], &rji[..len]);
    const L: usize = 4;
    let full = len / L * L;
    // L lanes per step as fixed-size arrays, which LLVM turns into SIMD; each lane is the
    // scalar `upd`.
    let lane = |v: &mut [f64],
                w: &mut [f64],
                c: (f64, f64, f64, f64),
                tr: &[f64],
                ti: &[f64],
                r0: &[f64],
                r1: &[f64]| {
        let (tir, tii, rir, rii) = c;
        let mut nr = [0.0f64; L];
        let mut ni = [0.0f64; L];
        for x in 0..L {
            let ar = tr[x] * rir - ti[x] * rii;
            let ai = tr[x] * rii + ti[x] * rir;
            let br = tir * r0[x] - tii * r1[x];
            let bi = tir * r1[x] + tii * r0[x];
            nr[x] = v[x] + (ar - br);
            ni[x] = w[x] + (ai - bi);
        }
        v[..L].copy_from_slice(&nr);
        w[..L].copy_from_slice(&ni);
    };
    let mut idx = 0;
    while idx < full {
        let e = idx + L;
        let (tr, ti, r0, r1) = (&tjr[idx..e], &tji[idx..e], &rjr[idx..e], &rji[idx..e]);
        lane(&mut ra[idx..e], &mut ia[idx..e], ci, tr, ti, r0, r1);
        lane(&mut rb[idx..e], &mut ib[idx..e], cn, tr, ti, r0, r1);
        idx = e;
    }
    while idx < len {
        let tj = (tjr[idx], tji[idx]);
        let rj = (rjr[idx], rji[idx]);
        upd(&mut ra[idx], &mut ia[idx], ci, tj, rj);
        upd(&mut rb[idx], &mut ib[idx], cn, tj, rj);
        idx += 1;
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    fn lcg(s: &mut u64) -> f64 {
        *s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*s >> 11) as f64 / (1u64 << 53) as f64) - 0.5
    }

    /// `cargo test --release --lib fastsim::kernels::bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn time_kernels() {
        let (m, n) = (216usize, 108usize);
        let mut s = 7u64;
        let ys: Vec<Tv> = (0..m)
            .map(|i| Tv {
                odd: i % 2 == 1,
                v: (0..n).map(|_| lcg(&mut s)).collect(),
            })
            .collect();
        let refs: Vec<&Tv> = ys.iter().collect();
        let t = std::time::Instant::now();
        let mut h = 0.0;
        for _ in 0..50 {
            h += std::hint::black_box(vacuum_expectation(&refs)).map_or(0.0, |c| c.re);
        }
        eprintln!(
            "vacuum_expectation: {:.3} ms ({h})",
            t.elapsed().as_secs_f64() * 1e3 / 50.0
        );
        let t = std::time::Instant::now();
        let mut x = 0.0;
        for i in 0..11_600 {
            x += std::hint::black_box(f64::from(i) * 1e-3).hypot(0.5);
        }
        eprintln!(
            "11.6k hypot: {:.3} ms ({x})",
            t.elapsed().as_secs_f64() * 1e3
        );
        let u: Vec<f64> = (0..n * n).map(|_| lcg(&mut s)).collect();
        let t = std::time::Instant::now();
        for _ in 0..20 {
            h += std::hint::black_box(ad_residual(Some(&u), &refs, n));
        }
        eprintln!(
            "ad_residual: {:.3} ms ({h})",
            t.elapsed().as_secs_f64() * 1e3 / 20.0
        );
    }
}
