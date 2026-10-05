//! The reference Pfaffian elimination (`crate::sim::gaussian::pfaffian`) on *real
//! coefficients*: the same float operations on every non-zero component the reference computes,
//! with the delayed updates of `pfblock`, and without the operations whose result is a zero.
//!
//! # Phase classes
//!
//! Give index `x` the parity `par[x]` of its tracked vector (`Tv::odd`). The reference's
//! contraction puts `d` in the real part of entry `(i, j)` when `par[i] == par[j]`, and `d`
//! (even, odd) or `-d` (odd, even) in the imaginary part otherwise; the other part is the
//! literal `0.0`. Call `(i, j)` *real-class* or *imaginary-class* accordingly and its non-zero
//! part its *coefficient*. Claim: through the whole elimination, every entry the reference reads
//! has its off-class part equal to a zero, and its coefficient is obtained from the coefficients
//! of the values it is computed from by the following real operations, each the one operation
//! of the reference that produces the non-zero part (`s` = -1 or 1 is an exact negation):
//!
//! - **`C64::mul`** of classes `a`, `b` gives class `a xor b`, coefficient `s (x y)` with
//!   `s = -1` iff both are imaginary-class: the non-zero part is `x y +- 0 0` or (imaginary
//!   times imaginary) `0 0 - x y`, and `p + z = p`, `z - p = -p` for a zero `z` and non-zero
//!   `p` (a zero `p` gives a zero either way). The off-class part is a sum or difference of two
//!   products with a zero factor: a zero.
//! - **`C64::div`** by the head (class `h`) of an entry of class `a`: class `a xor h`,
//!   coefficient `s ((x y) / (y y))` with `s = -1` iff `a` is real-class and `h`
//!   imaginary-class (the part is `(0 0 - x y) / d`, and `(-p) / d = -(p / d)`); `d` is
//!   `y y + 0 0 = y y`.
//! - **`C64::sub`, `+=`** of equal classes: the coefficients' difference or sum, and a zero.
//! - **`C64::norm`** is `hypot(c, z)` = `|c|` exactly (the identity `pfblock` uses).
//!
//! Classes compose consistently: at step `k`, `tau_j` has class `par[j] xor par[k+1]`, `row_i`
//! class `par[i] xor par[k+1]`, so both products of the update of `(i, j)` have class
//! `par[i] xor par[j]`, the entry's own. With `q_x = (par[x] != par[k+1])` the update of the
//! coefficient is `s_ij (t_j r_i - t_i r_j)` with `s_ij = -1` iff `q_i` and `q_j` (both products
//! carry the same sign, and `(-a) - (-b) = -(a - b)` exactly): the kernel stores, per step, `t`
//! and `r` and their copies negated where `q_j` holds, and row `i` uses the negated copies iff
//! `q_i` (`(-t) r = -(t r)`, `-a - (-b) = -(a - b)`). By induction from the input, every value
//! read (pivot norms, head, `tau`, `row`) and every entry equals the reference's up to zero
//! signs, and the Pfaffian (`C64::mul` of the heads, as the reference, on the reconstructed
//! complex heads) is the reference's up to zero signs; the pivot choices are the reference's
//! (`pfblock` module docs). A swap moves a position's parity with its entries.
//!
//! The finiteness rules are `pfblock`'s: `None` when a value read is not finite. A zero head
//! returns zero, as the reference.
//!
//! Storage: the upper triangle of the coefficients, row-major with the row stride `m` rounded
//! up to a multiple of 8 (`mp`). The delayed updates are `pfblock`'s (every live entry stale by
//! the pending panel; rows `k`, `k + 1` brought up to date when read; a flush when the panel is
//! full). A row's update runs over whole 8-column blocks, from the block holding column `x + 1`
//! to `mp`: the extra columns (`<= x`, the lower triangle, and `>= m`, padding) are scratch that
//! nothing reads (the pivot search, `tau`, `row` and the swap read upper entries only), so
//! updating them changes no value read.
use crate::sim::gaussian::pfaffian::C64;

/// Steps per panel.
pub const PANEL: usize = 16;
/// The largest panel.
pub const MAX_PANEL: usize = 32;

fn cmul(a: C64, o: C64) -> C64 {
    C64 {
        re: a.re * o.re - a.im * o.im,
        im: a.re * o.im + a.im * o.re,
    }
}

/// The coefficient matrix of the reference's contraction matrix whose upper triangle is `re`,
/// `im`, given the parities: `None` unless every off-class part is a zero (tests, tooling).
#[must_use]
pub fn coefficients(re: &[f64], im: &[f64], par: &[bool]) -> Option<Vec<f64>> {
    let m = par.len();
    let mut c = vec![0.0; m * m];
    for i in 0..m {
        for j in i + 1..m {
            let (a, b) = (re[i * m + j], im[i * m + j]);
            let (v, z) = if par[i] == par[j] { (a, b) } else { (b, a) };
            if z != 0.0 {
                return None;
            }
            c[i * m + j] = v;
        }
    }
    Some(c)
}

/// Pending steps: step `s`'s vectors at `p[(4 s + v) m + x]`, `v` = 0 `t`, 1 `r`, 2 `t`
/// negated where `q`, 3 `r` negated where `q`; `pk1[s]` the parity of position `k + 1` at that
/// step.
struct Pending {
    p: Vec<f64>,
    /// The row and vector stride (`m` rounded up to a multiple of 8).
    mp: usize,
    pk1: [bool; MAX_PANEL],
    m: usize,
    n: usize,
}

/// The reference's vacuum overlap from the coefficient matrix `c` (upper triangle, destroyed)
/// and the parities `par` (destroyed), `panel` steps per panel (module docs).
#[must_use]
pub fn pfaffian_real(c: &mut [f64], par: &mut [bool], m: usize, panel: usize) -> Option<C64> {
    let panel = panel.clamp(1, MAX_PANEL);
    let mp = m.div_ceil(8) * 8;
    thread_local! {
        static BUF: std::cell::RefCell<(Vec<f64>, Vec<f64>)> =
            const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
    }
    let (mut cp, mut p) = BUF.with(|b| std::mem::take(&mut *b.borrow_mut()));
    cp.clear();
    cp.resize(m * mp, 0.0);
    for i in 0..m {
        cp[i * mp + i + 1..i * mp + m].copy_from_slice(&c[i * m + i + 1..i * m + m]);
    }
    p.clear();
    p.resize(panel * 4 * mp, 0.0);
    let mut pd = Pending {
        p,
        mp,
        pk1: [false; MAX_PANEL],
        m,
        n: 0,
    };
    let r = eliminate(&mut cp, par, &mut pd, panel);
    BUF.with(|b| *b.borrow_mut() = (cp, pd.p));
    r
}

/// The elimination loop of [`pfaffian_real`] on the padded storage.
fn eliminate(c: &mut [f64], par: &mut [bool], pd: &mut Pending, panel: usize) -> Option<C64> {
    let (m, mp) = (pd.m, pd.mp);
    let mut pf = C64 { re: 1.0, im: 0.0 };
    for k in (0..m).step_by(2) {
        if k + 1 >= m {
            break;
        }
        update_row(c, par, pd, k);
        let piv = pivot(&c[k * mp + k + 1..k * mp + m])? + k + 1;
        if piv != k + 1 {
            swap(c, par, pd, k, k + 1, piv);
            pf = C64 {
                re: -pf.re,
                im: -pf.im,
            };
        }
        let h = c[k * mp + k + 1];
        if h == 0.0 {
            // `hypot` of the head is `|h|` (module docs).
            return Some(C64 { re: 0.0, im: 0.0 });
        }
        let head_imag = par[k] != par[k + 1];
        let head = if head_imag {
            C64 { re: 0.0, im: h }
        } else {
            C64 { re: h, im: 0.0 }
        };
        pf = cmul(pf, head);
        if k + 2 >= m {
            break;
        }
        update_row(c, par, pd, k + 1);
        if !push_step(c, par, pd, k, h, head_imag) {
            return None;
        }
        if pd.n == panel {
            for i in k + 2..m - 1 {
                update_row(c, par, pd, i);
            }
            pd.n = 0;
        }
    }
    (pf.re.is_finite() && pf.im.is_finite()).then_some(pf)
}

/// Step `k`'s `tau` and `row` coefficients at positions `k + 2..` (module docs), appended to
/// the pending steps; `false` when any is not finite.
fn push_step(c: &[f64], par: &[bool], pd: &mut Pending, k: usize, h: f64, head_imag: bool) -> bool {
    let (m, mp) = (pd.m, pd.mp);
    let s = pd.n;
    let lo = k + 2;
    let d = h * h;
    let pk1 = par[k + 1];
    let (t, rest) = pd.p[4 * s * mp..(4 * s + 4) * mp].split_at_mut(mp);
    let (r, rest) = rest.split_at_mut(mp);
    let (tn, rn) = rest.split_at_mut(mp);
    let ak = &c[k * mp..k * mp + m];
    let ar = &c[(k + 1) * mp..(k + 1) * mp + m];
    let mut ok = true;
    for j in lo..m {
        let num = ak[j] * h;
        // Entry (k, j) real-class and the head imaginary-class: `(0 0 - x y) / d`.
        let tj = if head_imag && par[k] == par[j] {
            (-num) / d
        } else {
            num / d
        };
        let rj = ar[j];
        ok &= tj.is_finite() && rj.is_finite();
        t[j] = tj;
        r[j] = rj;
        if par[j] != pk1 {
            tn[j] = -tj;
            rn[j] = -rj;
        } else {
            tn[j] = tj;
            rn[j] = rj;
        }
    }
    pd.pk1[s] = pk1;
    pd.n += 1;
    ok
}

/// The reference's pivot in a row of coefficients: the last index of the maximum of `|c|`;
/// `None` when one is not finite.
fn pivot(row: &[f64]) -> Option<usize> {
    let mut best = -1.0f64;
    let mut at = 0;
    for (j, &x) in row.iter().enumerate() {
        let a = x.abs();
        if !a.is_finite() {
            return None;
        }
        if a >= best {
            best = a;
            at = j;
        }
    }
    Some(at)
}

/// Exchanges positions `x < y` (live positions `>= k`; row `k` up to date, every other live
/// entry stale by the pending steps), with the pending vectors and the parities.
fn swap(c: &mut [f64], par: &mut [bool], pd: &mut Pending, k: usize, x: usize, y: usize) {
    let (m, mp) = (pd.m, pd.mp);
    c.swap(k * mp + x, k * mp + y);
    for j in y + 1..m {
        c.swap(x * mp + j, y * mp + j);
    }
    for j in x + 1..y {
        let (axj, ajy) = (c[x * mp + j], c[j * mp + y]);
        c[x * mp + j] = -ajy;
        c[j * mp + y] = -axj;
    }
    c[x * mp + y] = -c[x * mp + y];
    par.swap(x, y);
    for s in 0..pd.n {
        for v in 0..4 {
            pd.p.swap((4 * s + v) * mp + x, (4 * s + v) * mp + y);
        }
    }
}

/// Row `x`'s entries `(x, j)`, `j > x`, receive the pending steps in order:
/// `c += t_j r_x - t_x r_j` with the copies negated where `q` when `q_x` (module docs). The
/// update runs over the whole 8-column blocks from the one holding `x + 1` to `mp` (module docs:
/// the extra columns are scratch).
fn update_row(c: &mut [f64], par: &[bool], pd: &Pending, x: usize) {
    let (m, mp) = (pd.m, pd.mp);
    if pd.n == 0 || x + 1 >= m {
        return;
    }
    // Per step: `t_x`, `r_x` and the offset of the vectors this row uses.
    let mut sc = [(0.0f64, 0.0f64, 0usize); MAX_PANEL];
    for (s, e) in sc.iter_mut().enumerate().take(pd.n) {
        let q = par[x] != pd.pk1[s];
        let base = (4 * s + if q { 2 } else { 0 }) * mp;
        *e = (pd.p[4 * s * mp + x], pd.p[(4 * s + 1) * mp + x], base);
    }
    let sc = &sc[..pd.n];
    let lo = (x + 1) / 8 * 8;
    let row = &mut c[x * mp + lo..x * mp + mp];
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `lo + row.len() == mp`, a multiple of 8; every pending vector holds `mp`
        // values.
        unsafe { neon::kernel(row, &pd.p, mp, lo, sc) };
    }
    #[cfg(not(target_arch = "aarch64"))]
    for (t, e) in row.iter_mut().enumerate() {
        let j = lo + t;
        let mut a = *e;
        for &(tx, rx, base) in sc {
            let (tj, rj) = (pd.p[base + j], pd.p[base + mp + j]);
            a += tj * rx - tx * rj;
        }
        *e = a;
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::{
        float64x2_t, vaddq_f64, vdupq_n_f64, vld1q_f64, vmulq_f64, vst1q_f64, vsubq_f64,
    };

    /// `W` two-lane registers (`2 W` entries) of `super::update_row`'s loop: for each step in
    /// order, `a += (t_j r_x) - (t_x r_j)`, the reference's operation order.
    #[inline(always)]
    unsafe fn block<const W: usize>(
        e: *mut f64,
        tb: *const f64,
        m: usize,
        sc: &[(f64, f64, usize)],
    ) {
        let mut a: [float64x2_t; W] = [vdupq_n_f64(0.0); W];
        for (w, r) in a.iter_mut().enumerate() {
            *r = vld1q_f64(e.add(2 * w));
        }
        for &(tx, rx, base) in sc {
            let tp = tb.add(base);
            let rp = tp.add(m);
            let (vtx, vrx) = (vdupq_n_f64(tx), vdupq_n_f64(rx));
            for (w, r) in a.iter_mut().enumerate() {
                let x = vmulq_f64(vld1q_f64(tp.add(2 * w)), vrx);
                let y = vmulq_f64(vtx, vld1q_f64(rp.add(2 * w)));
                *r = vaddq_f64(*r, vsubq_f64(x, y));
            }
        }
        for (w, r) in a.iter().enumerate() {
            vst1q_f64(e.add(2 * w), *r);
        }
    }

    /// `super::update_row`'s loop on the whole row slice, sixteen entries at a time, then eight.
    ///
    /// # Safety
    /// `row.len()` is a multiple of 8 and `off + row.len() <= m`; `p` holds every
    /// `base + m + j` read (`base` from `sc`).
    #[inline]
    pub unsafe fn kernel(
        row: &mut [f64],
        p: &[f64],
        m: usize,
        off: usize,
        sc: &[(f64, f64, usize)],
    ) {
        let len = row.len();
        let pb = p.as_ptr().add(off);
        let e = row.as_mut_ptr();
        let mut j = 0;
        while j + 16 <= len {
            block::<8>(e.add(j), pb.add(j), m, sc);
            j += 16;
        }
        if j + 8 <= len {
            block::<4>(e.add(j), pb.add(j), m, sc);
        }
    }
}
