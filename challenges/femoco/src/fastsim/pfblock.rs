//! The reference Pfaffian elimination (`crate::sim::gaussian::pfaffian`) with *delayed*
//! updates and *linked columns*: the same float operations on every entry that is computed, in
//! the same order, applied a panel of steps at a time; and an entry that the reference computes
//! as an exact multiple of another entry by `w` in `{1, i, -1, -i}` is not computed at all
//! until it is read, when it is that multiple.
//!
//! # Delayed updates
//!
//! The reference's step `k` (pivot search in row `k`, swap of `k + 1` and the pivot, `head`,
//! then `A_ij += tau_j row_i - tau_i row_j` for every `i, j >= k + 2`) reads only rows `k` and
//! `k + 1`, and the update of an entry depends only on that entry and on `tau`, `row` at its
//! two indices. So an entry's value after step `k` is its initial value followed by the updates
//! of steps `0, 2, ..., k` in that order, each computed from that step's `tau` and `row`. This
//! kernel keeps every live entry *stale by the same set of steps* (the pending panel, whose
//! `tau`, `row` it stores) and brings an entry up to date only when it is read:
//!
//! - at step `k`, row `k` (and, after the swap, row `k + 1`) is brought up to date by applying
//!   the pending steps in order; those two rows are then eliminated and never read again;
//! - when the panel is full, every live entry receives the pending steps in order (`flush`).
//!
//! No entry receives a step twice (the rows brought up to date are the eliminated ones, which a
//! flush skips) or misses one (an entry is read only in rows `k`, `k + 1`). The swap of
//! positions `k + 1` and `piv` exchanges entries that are stale by the same steps, together with
//! the pending `tau`, `row` at those positions, and negates an entry whose orientation flips; the
//! update of the flipped orientation is the negated update (IEEE subtraction is
//! sign-symmetric), so the negated stale entry plus the negated updates is the negated up-to-date
//! entry, up to the sign of a zero.
//!
//! # Linked columns
//!
//! Column `p + 1` is *linked* to column `p` with factor `w` (`link[p] = Some(w)`) when, for
//! every live row `x < p`, the reference's `A(x, p + 1)` equals `w A(x, p)` up to zero sign. A
//! pair's two factors (the same vector in both blocks, `kernels` docs) give such columns: the
//! contraction of `y_x` with `(v, even)` and with `(v, odd)` differ by the factor `i` exactly.
//! The link is established by comparing the initial entries themselves (no assumption about
//! where they came from), and it survives step `k` when that step's `tau` and `row` satisfy
//! `tau_{p+1} = w tau_p` and `row_{p+1} = w row_p` (checked on the values, up to zero sign):
//! the update of `(x, p + 1)` is then `tau_{p+1} row_x - tau_x row_{p+1}`
//! `= (w tau_p) row_x - tau_x (w row_p)`, and for `w` in `{1, i, -1, -i}` multiplying by `w`
//! only swaps and negates components, which commutes exactly with the reference's complex
//! product (`(w a) b` and `a (w b)` both equal `w (a b)` component for component) and with
//! sums and differences, up to zero sign. So the update is `w` times the update of `(x, p)`,
//! and `w A(x, p) + w u = w (A(x, p) + u)` up to zero sign.
//!
//! A link that fails the check, or whose columns a swap moves, is dropped for good. While a link
//! holds, the entries of column `p + 1` in rows above `p` are not computed: whenever the entries
//! of column `p` in such a row receive steps (bringing a row up to date, or a flush), the entry
//! in column `p + 1` is *set* to `w` times the new entry of column `p`, which is the value the
//! reference computes (the stored entry is stale by the same steps, all of which passed the
//! check). Every stored entry therefore always holds the reference's value as of the steps it
//! has received, and dropping a link needs no repair. Entry `(p, p + 1)` itself (in no row
//! above `p`) is always computed.
//!
//! Hence every value read (pivot norms, `head`, `tau`, `row`) and the result equal the
//! reference's up to zero signs (`kernels` module docs), and the pivot choices (the last
//! maximum of the norms, as `max_by` with `total_cmp`) are the reference's.
//!
//! Storage is the upper triangle only, row-major `m x m` (the lower entry is the negated upper
//! entry up to zero sign, `kernels` module docs). `None` when a value read is not finite.
#![allow(clippy::needless_range_loop)]
use crate::sim::gaussian::pfaffian::C64;

/// Steps per panel (a performance choice; tests run several).
pub const PANEL: usize = 16;

fn cmul(a: C64, o: C64) -> C64 {
    C64 {
        re: a.re * o.re - a.im * o.im,
        im: a.re * o.im + a.im * o.re,
    }
}

/// `w^k z` for `w = i` (`k` in 0..4): components swapped and negated, exactly.
#[inline]
fn mulw(re: f64, im: f64, k: u8) -> (f64, f64) {
    match k & 3 {
        0 => (re, im),
        1 => (-im, re),
        2 => (-re, -im),
        _ => (im, -re),
    }
}

#[inline]
fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a == 0.0 && b == 0.0)
}

/// Counters for tests and the notes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    /// Upper-triangle entry updates the reference performs.
    pub updates: u64,
    /// Of those, the ones this kernel did not compute (linked columns).
    pub skipped: u64,
    /// Links found in the input.
    pub links: u64,
}

/// The largest panel (scalar buffers are fixed arrays).
pub const MAX_PANEL: usize = 32;

/// Pending steps: for step `s`, `p[(4 s + c) m + x]` with `c` = 0 `tau` real, 1 `tau`
/// imaginary, 2 `row` real, 3 `row` imaginary, at position `x`.
struct Pending {
    p: Vec<f64>,
    m: usize,
    n: usize,
}

/// The reference's Pfaffian of the antisymmetric matrix whose upper triangle is `re`, `im`
/// (destroyed), with `panel` steps per panel (module docs).
#[must_use]
pub fn pfaffian_dense(re: &mut [f64], im: &mut [f64], m: usize, panel: usize) -> Option<C64> {
    pfaffian_dense_counted(re, im, m, panel, true, None)
}

/// [`pfaffian_dense`] with the linked-column switch (`use_links` false: every entry is
/// computed) and counters.
#[must_use]
pub fn pfaffian_dense_counted(
    re: &mut [f64],
    im: &mut [f64],
    m: usize,
    panel: usize,
    use_links: bool,
    mut counts: Option<&mut Counts>,
) -> Option<C64> {
    let panel = panel.clamp(1, MAX_PANEL);
    let mut pf = C64 { re: 1.0, im: 0.0 };
    let mut pd = Pending {
        p: vec![0.0; panel * 4 * m],
        m,
        n: 0,
    };
    let mut link: Vec<Option<u8>> = vec![None; m];
    if use_links {
        find_links(re, im, m, &mut link);
        if let Some(c) = counts.as_deref_mut() {
            c.links += link.iter().filter(|l| l.is_some()).count() as u64;
        }
    }
    let mut ws = Work::default();
    for k in (0..m).step_by(2) {
        if k + 1 >= m {
            break;
        }
        // A link at p means something while a live row lies above p.
        for l in &mut link[..(k + 1).min(m)] {
            *l = None;
        }
        segments(&link, k + 1, k + 1, m, &mut ws.segs);
        update_row(re, im, &pd, k, &ws.segs, None);
        let piv = pivot(
            &re[k * m + k + 1..k * m + m],
            &im[k * m + k + 1..k * m + m],
            &mut ws.norms,
        )? + k
            + 1;
        if piv != k + 1 {
            for p in [k + 1, piv - 1, piv] {
                if p < m {
                    link[p] = None;
                }
            }
            swap(re, im, &mut pd, k, k + 1, piv);
            pf = C64 {
                re: -pf.re,
                im: -pf.im,
            };
            // Row k + 1 is now the pivot's row, which lies above only the links beyond the
            // pivot: columns up to the pivot are computed in it.
            segments(&link, k + 1, piv + 1, m, &mut ws.segs);
        }
        let head = C64 {
            re: re[k * m + k + 1],
            im: im[k * m + k + 1],
        };
        if head.re.hypot(head.im) == 0.0 {
            return Some(C64 { re: 0.0, im: 0.0 });
        }
        pf = cmul(pf, head);
        if k + 2 >= m {
            break;
        }
        update_row(re, im, &pd, k + 1, &ws.segs, None);
        let s = pd.n;
        if !push_step(re, im, &mut pd, k, head) {
            return None;
        }
        // The links that survive this step (module docs).
        for p in k + 2..m.saturating_sub(1) {
            let Some(w) = link[p] else { continue };
            let at = |c: usize, x: usize| pd.p[(4 * s + c) * m + x];
            let (tr, ti) = mulw(at(0, p), at(1, p), w);
            let (rr, ri) = mulw(at(2, p), at(3, p), w);
            if !(same(at(0, p + 1), tr)
                && same(at(1, p + 1), ti)
                && same(at(2, p + 1), rr)
                && same(at(3, p + 1), ri))
            {
                link[p] = None;
            }
        }
        if let Some(c) = counts.as_deref_mut() {
            let live = (m - k - 2) as u64;
            c.updates += live * live.saturating_sub(1) / 2;
            for p in k + 2..m.saturating_sub(1) {
                if link[p].is_some() {
                    // Rows k + 2 .. p skip column p + 1.
                    c.skipped += (p - (k + 2)) as u64;
                }
            }
        }
        if pd.n == panel {
            flush(re, im, &pd, k + 2, &link, &mut ws);
            pd.n = 0;
        }
    }
    (pf.re.is_finite() && pf.im.is_finite()).then_some(pf)
}

/// Step `k`'s `tau` (row `k` over `head`, `cdiv` as the reference's `C64::div`) and `row`
/// (row `k + 1`) at positions `k + 2..`, appended to the pending steps; `false` when any is not
/// finite.
fn push_step(re: &[f64], im: &[f64], pd: &mut Pending, k: usize, head: C64) -> bool {
    let m = pd.m;
    let s = pd.n;
    let lo = k + 2;
    let d = head.re * head.re + head.im * head.im;
    let (hr, hi) = (head.re, head.im);
    let (tr, rest) = pd.p[4 * s * m..(4 * s + 4) * m].split_at_mut(m);
    let (ti, rest) = rest.split_at_mut(m);
    let (rr, ri) = rest.split_at_mut(m);
    let (ar, ai) = (&re[k * m + lo..k * m + m], &im[k * m + lo..k * m + m]);
    for (j, (&a, &b)) in ar.iter().zip(ai).enumerate() {
        tr[lo + j] = (a * hr + b * hi) / d;
        ti[lo + j] = (b * hr - a * hi) / d;
    }
    rr[lo..m].copy_from_slice(&re[(k + 1) * m + lo..(k + 1) * m + m]);
    ri[lo..m].copy_from_slice(&im[(k + 1) * m + lo..(k + 1) * m + m]);
    pd.n += 1;
    [&tr[lo..m], &ti[lo..m], &rr[lo..m], &ri[lo..m]]
        .iter()
        .all(|v| v.iter().all(|x| x.is_finite()))
}

/// The reference's pivot in a row: the last index of the maximum of `C64::norm` (`hypot`)
/// over the entries; `None` when a norm is not finite.
fn pivot(re: &[f64], im: &[f64], norms: &mut Vec<f64>) -> Option<usize> {
    // `|re| + |im|` is the norm when a part is zero (`norm2`); entries with both parts non-zero
    // are then given their `hypot`.
    norms.clear();
    norms.extend(re.iter().zip(im).map(|(a, b)| a.abs() + b.abs()));
    for (j, n) in norms.iter_mut().enumerate() {
        if re[j] != 0.0 && im[j] != 0.0 {
            *n = re[j].hypot(im[j]);
        }
    }
    let mut best = -1.0f64;
    for &n in norms.iter() {
        if !n.is_finite() {
            return None;
        }
        best = best.max(n);
    }
    norms.iter().rposition(|&n| n == best)
}

/// Greedy disjoint links on the input: `link[p] = Some(w)` when column `p + 1` is `w` times
/// column `p` in every row above `p` (`p >= 1`), checked entry by entry.
fn find_links(re: &[f64], im: &[f64], m: usize, link: &mut [Option<u8>]) {
    if m < 3 {
        return;
    }
    // Per column p: the factor so far (`UNSET` while every entry above was a zero pair).
    const UNSET: u8 = 4;
    let mut w = vec![UNSET; m];
    let mut ok = vec![false; m];
    // `k` fits entries `(a, b)` of column p and `(u, v)` of column p + 1.
    let fits = |k: u8, a: f64, b: f64, u: f64, v: f64| {
        let (c, d) = mulw(a, b, k);
        same(u, c) && same(v, d)
    };
    // Updates column p's verdict with one row's entries.
    let see = |w: &mut u8, ok: &mut bool, a: f64, b: f64, u: f64, v: f64| {
        if *w == UNSET {
            if a == 0.0 && b == 0.0 {
                *ok = u == 0.0 && v == 0.0;
            } else {
                match (0..4u8).find(|&k| fits(k, a, b, u, v)) {
                    Some(k) => *w = k,
                    None => *ok = false,
                }
            }
        } else if !fits(*w, a, b, u, v) {
            *ok = false;
        }
    };
    // Row 0 (above every p >= 1) picks the candidates.
    let mut cand = Vec::with_capacity(m);
    for p in 1..m - 1 {
        ok[p] = true;
        see(&mut w[p], &mut ok[p], re[p], im[p], re[p + 1], im[p + 1]);
        if ok[p] {
            cand.push(p);
        }
    }
    for x in 1..m - 2 {
        let row = x * m;
        let from = cand.partition_point(|&p| p <= x);
        for &p in &cand[from..] {
            if ok[p] {
                let (a, b) = (re[row + p], im[row + p]);
                let (u, v) = (re[row + p + 1], im[row + p + 1]);
                see(&mut w[p], &mut ok[p], a, b, u, v);
            }
        }
    }
    let mut p = 1;
    while p + 1 < m {
        if ok[p] {
            link[p] = Some(if w[p] == UNSET { 0 } else { w[p] });
            p += 2;
        } else {
            p += 1;
        }
    }
}

/// A run of columns `a..b`: every column computed (`w` none), or linked pairs `(a, a + 1)`,
/// `(a + 2, a + 3)`, ... with one factor `w` (`b - a` even; `q0`: the first pair's slot in
/// the flush's contiguous copy of the pending vectors).
#[derive(Clone, Copy)]
struct Seg {
    a: usize,
    b: usize,
    w: Option<u8>,
    q0: usize,
}

/// Buffers kept between steps.
#[derive(Default)]
struct Work {
    segs: Vec<Seg>,
    comp: Vec<f64>,
    norms: Vec<f64>,
}

/// The runs of columns `lo..m` under the links at `lf` (`>= lo`) and beyond.
fn segments(link: &[Option<u8>], lo: usize, lf: usize, m: usize, out: &mut Vec<Seg>) {
    out.clear();
    let starts = |j: usize| j >= lf && j + 1 < m && link[j].is_some();
    let mut j = lo;
    let mut q = 0;
    while j < m {
        let a = j;
        if starts(j) {
            let w = link[j];
            while starts(j) && link[j] == w {
                j += 2;
            }
            out.push(Seg { a, b: j, w, q0: q });
            q += (j - a) / 2;
        } else {
            j += 1;
            while j < m && !starts(j) {
                j += 1;
            }
            out.push(Seg {
                a,
                b: j,
                w: None,
                q0: 0,
            });
        }
    }
}

/// Exchanges positions `x < y` among the live positions (`>= k`, row `k` up to date, every
/// other live entry stale by the pending steps), with the pending vectors.
fn swap(re: &mut [f64], im: &mut [f64], pd: &mut Pending, k: usize, x: usize, y: usize) {
    let m = pd.m;
    for a in [&mut *re, &mut *im] {
        a.swap(k * m + x, k * m + y);
        for j in y + 1..m {
            a.swap(x * m + j, y * m + j);
        }
        for j in x + 1..y {
            let (axj, ajy) = (a[x * m + j], a[j * m + y]);
            a[x * m + j] = -ajy;
            a[j * m + y] = -axj;
        }
        a[x * m + y] = -a[x * m + y];
    }
    for s in 0..pd.n {
        for c in 0..4 {
            pd.p.swap((4 * s + c) * m + x, (4 * s + c) * m + y);
        }
    }
}

/// Every live row `from..` receives the pending steps (`update_row`).
fn flush(
    re: &mut [f64],
    im: &mut [f64],
    pd: &Pending,
    from: usize,
    link: &[Option<u8>],
    ws: &mut Work,
) {
    let m = pd.m;
    segments(link, from, from, m, &mut ws.segs);
    // The pending vectors at the linked runs' first columns, contiguous by slot.
    let slots: usize = ws
        .segs
        .iter()
        .filter(|g| g.w.is_some())
        .map(|g| (g.b - g.a) / 2)
        .sum();
    ws.comp.clear();
    ws.comp.resize(pd.n * 4 * slots, 0.0);
    for g in ws.segs.iter().filter(|g| g.w.is_some()) {
        for s in 0..pd.n {
            for c in 0..4 {
                let src = &pd.p[(4 * s + c) * m..(4 * s + c + 1) * m];
                let dst = &mut ws.comp[(4 * s + c) * slots..(4 * s + c + 1) * slots];
                for t in 0..(g.b - g.a) / 2 {
                    dst[g.q0 + t] = src[g.a + 2 * t];
                }
            }
        }
    }
    let comp = Compact { p: &ws.comp, slots };
    for i in from..m - 1 {
        update_row(re, im, pd, i, &ws.segs, Some(&comp));
    }
}

/// The flush's contiguous pending vectors of the linked runs: step `s`, component `c`, slot
/// `q` at `p[(4 s + c) slots + q]`.
struct Compact<'a> {
    p: &'a [f64],
    slots: usize,
}

/// Row `x`'s entries `(x, j)`, `j > x`, receive the pending steps in order (`A_xj += tau_j
/// row_x - tau_x row_j`, `C64::mul`, `C64::sub`, then `+=`, as the reference); in a linked run
/// above `x` the second column of each pair is set from the first instead (module docs).
fn update_row(
    re: &mut [f64],
    im: &mut [f64],
    pd: &Pending,
    x: usize,
    segs: &[Seg],
    comp: Option<&Compact<'_>>,
) {
    let m = pd.m;
    if pd.n == 0 || x + 1 >= m {
        return;
    }
    let mut buf = [[0.0f64; 4]; MAX_PANEL];
    for (s, b) in buf.iter_mut().enumerate().take(pd.n) {
        *b = std::array::from_fn(|c| pd.p[(4 * s + c) * m + x]);
    }
    let sc = &buf[..pd.n];
    let row = x * m;
    for g in segs {
        if g.b <= x + 1 {
            continue;
        }
        let mut a = g.a.max(x + 1);
        let Some(w) = g.w else {
            kernel(
                &mut re[row + a..row + g.b],
                &mut im[row + a..row + g.b],
                pd,
                a,
                sc,
            );
            continue;
        };
        if (a - g.a) % 2 == 1 {
            // Column a is the second of the pair (x, a): the entry is computed.
            kernel(
                &mut re[row + a..=row + a],
                &mut im[row + a..=row + a],
                pd,
                a,
                sc,
            );
            a += 1;
        }
        let pairs = (g.b - a) / 2;
        if pairs == 0 {
            continue;
        }
        let (vr, vi) = (&mut re[row + a..row + g.b], &mut im[row + a..row + g.b]);
        match comp {
            Some(c) => {
                let q = g.q0 + (a - g.a) / 2;
                linked(vr, vi, pairs, &c.p[q..], c.slots, 1, sc, w);
            }
            None => linked(vr, vi, pairs, &pd.p[a..], m, 2, sc, w),
        }
    }
}

/// Pairs `t` of a linked run: `v[2 t] += ...` with step `s`'s `tau`, `row` of the pair's first
/// column at `cols[(4 s + c) stride + step t]`, then `v[2 t + 1] = w v[2 t]`.
#[allow(clippy::too_many_arguments)]
#[inline]
fn linked(
    vr: &mut [f64],
    vi: &mut [f64],
    pairs: usize,
    cols: &[f64],
    stride: usize,
    step: usize,
    sc: &[[f64; 4]],
    w: u8,
) {
    #[cfg(target_arch = "aarch64")]
    let done = {
        // SAFETY: `vr`, `vi` hold `2 pairs` entries; `cols` holds every `(4 s + c) stride +
        // step t` read (`update_row`).
        unsafe { neon::linked(vr, vi, pairs, cols, stride, step, sc, w) }
    };
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    for t in done..pairs {
        let (mut ar, mut ai) = (vr[2 * t], vi[2 * t]);
        for (s, &[tir, tii, rir, rii]) in sc.iter().enumerate() {
            let at = |c: usize| cols[(4 * s + c) * stride + step * t];
            let (tjr, tji, rjr, rji) = (at(0), at(1), at(2), at(3));
            let xr = tjr * rir - tji * rii;
            let xi = tjr * rii + tji * rir;
            let yr = tir * rjr - tii * rji;
            let yi = tir * rji + tii * rjr;
            ar += xr - yr;
            ai += xi - yi;
        }
        vr[2 * t] = ar;
        vi[2 * t] = ai;
        let (dr, di) = mulw(ar, ai, w);
        vr[2 * t + 1] = dr;
        vi[2 * t + 1] = di;
    }
}

/// `v[t] += tau_j row_i - tau_i row_j` for each pending step in order (`C64::mul`, `C64::sub`,
/// then `+=`, as the reference), where step `s`'s `tau_j`, `row_j` are the pending vectors at
/// position `off + t` and `tau_i`, `row_i` are `sc[s]`.
#[inline]
fn kernel(vr: &mut [f64], vi: &mut [f64], pd: &Pending, off: usize, sc: &[[f64; 4]]) {
    let len = vr.len().min(vi.len());
    #[cfg(target_arch = "aarch64")]
    let done = {
        // SAFETY: `off + len <= m` (callers), every pending vector holds `m`.
        unsafe { neon::kernel(vr, vi, &pd.p, pd.m, off, sc) }
    };
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    let st = pd.m;
    for t in done..len {
        let (mut ar, mut ai) = (vr[t], vi[t]);
        for (s, &[tir, tii, rir, rii]) in sc.iter().enumerate() {
            let j = off + t;
            let (tjr, tji, rjr, rji) = (
                pd.p[4 * s * st + j],
                pd.p[(4 * s + 1) * st + j],
                pd.p[(4 * s + 2) * st + j],
                pd.p[(4 * s + 3) * st + j],
            );
            let xr = tjr * rir - tji * rii;
            let xi = tjr * rii + tji * rir;
            let yr = tir * rjr - tii * rji;
            let yi = tir * rji + tii * rjr;
            ar += xr - yr;
            ai += xi - yi;
        }
        vr[t] = ar;
        vi[t] = ai;
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::{
        float64x2_t, vaddq_f64, vdupq_n_f64, vld1q_f64, vmulq_f64, vnegq_f64, vst1q_f64, vsubq_f64,
        vuzp1q_f64, vzip1q_f64, vzip2q_f64,
    };

    /// One step on two lanes: `v += (tau_j row_i) - (tau_i row_j)` with the reference's
    /// operations (scalars `i` broadcast).
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    unsafe fn step(
        (tir, tii, rir, rii): (float64x2_t, float64x2_t, float64x2_t, float64x2_t),
        tjr: float64x2_t,
        tji: float64x2_t,
        rjr: float64x2_t,
        rji: float64x2_t,
        vr: float64x2_t,
        vi: float64x2_t,
    ) -> (float64x2_t, float64x2_t) {
        let xr = vsubq_f64(vmulq_f64(tjr, rir), vmulq_f64(tji, rii));
        let xi = vaddq_f64(vmulq_f64(tjr, rii), vmulq_f64(tji, rir));
        let yr = vsubq_f64(vmulq_f64(tir, rjr), vmulq_f64(tii, rji));
        let yi = vaddq_f64(vmulq_f64(tir, rji), vmulq_f64(tii, rjr));
        (
            vaddq_f64(vr, vsubq_f64(xr, yr)),
            vaddq_f64(vi, vsubq_f64(xi, yi)),
        )
    }

    #[inline(always)]
    unsafe fn dup(s: &[f64; 4]) -> (float64x2_t, float64x2_t, float64x2_t, float64x2_t) {
        (
            vdupq_n_f64(s[0]),
            vdupq_n_f64(s[1]),
            vdupq_n_f64(s[2]),
            vdupq_n_f64(s[3]),
        )
    }

    /// `super::kernel` on four entries at a time; returns how many leading entries it did.
    ///
    /// # Safety
    /// `off + vr.len() <= st`; `p` holds `4 sc.len() st` values.
    #[inline]
    pub unsafe fn kernel(
        vr: &mut [f64],
        vi: &mut [f64],
        p: &[f64],
        st: usize,
        off: usize,
        sc: &[[f64; 4]],
    ) -> usize {
        let len = vr.len().min(vi.len());
        let full = len / 4 * 4;
        let base = p.as_ptr();
        let mut j = 0;
        while j < full {
            let pr = vr.as_mut_ptr().add(j);
            let pi = vi.as_mut_ptr().add(j);
            let mut r0 = vld1q_f64(pr);
            let mut r1 = vld1q_f64(pr.add(2));
            let mut i0 = vld1q_f64(pi);
            let mut i1 = vld1q_f64(pi.add(2));
            for (s, sv) in sc.iter().enumerate() {
                let sb = base.add(4 * s * st + off + j);
                let d = dup(sv);
                (r0, i0) = step(
                    d,
                    vld1q_f64(sb),
                    vld1q_f64(sb.add(st)),
                    vld1q_f64(sb.add(2 * st)),
                    vld1q_f64(sb.add(3 * st)),
                    r0,
                    i0,
                );
                (r1, i1) = step(
                    d,
                    vld1q_f64(sb.add(2)),
                    vld1q_f64(sb.add(st + 2)),
                    vld1q_f64(sb.add(2 * st + 2)),
                    vld1q_f64(sb.add(3 * st + 2)),
                    r1,
                    i1,
                );
            }
            vst1q_f64(pr, r0);
            vst1q_f64(pr.add(2), r1);
            vst1q_f64(pi, i0);
            vst1q_f64(pi.add(2), i1);
            j += 4;
        }
        full
    }

    /// `w^k (re, im)` on two lanes, exactly.
    #[inline(always)]
    unsafe fn mulw(r: float64x2_t, i: float64x2_t, w: u8) -> (float64x2_t, float64x2_t) {
        match w & 3 {
            0 => (r, i),
            1 => (vnegq_f64(i), r),
            2 => (vnegq_f64(r), vnegq_f64(i)),
            _ => (i, vnegq_f64(r)),
        }
    }

    /// `super::linked` on four pairs at a time; returns how many leading pairs it did.
    ///
    /// # Safety
    /// `vr`, `vi` hold `2 pairs` entries; `cols` holds `(4 s + c) stride + step t + 1` for
    /// every step `s`, component `c`, pair `t < pairs` (the `+ 1` only when `step` is 2).
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub unsafe fn linked(
        vr: &mut [f64],
        vi: &mut [f64],
        pairs: usize,
        cols: &[f64],
        stride: usize,
        step_t: usize,
        sc: &[[f64; 4]],
        w: u8,
    ) -> usize {
        let full = pairs / 4 * 4;
        let base = cols.as_ptr();
        let load = |p: *const f64| -> float64x2_t {
            if step_t == 1 {
                vld1q_f64(p)
            } else {
                vuzp1q_f64(vld1q_f64(p), vld1q_f64(p.add(2)))
            }
        };
        let mut t = 0;
        while t < full {
            let pr = vr.as_mut_ptr().add(2 * t);
            let pi = vi.as_mut_ptr().add(2 * t);
            let mut r0 = vuzp1q_f64(vld1q_f64(pr), vld1q_f64(pr.add(2)));
            let mut r1 = vuzp1q_f64(vld1q_f64(pr.add(4)), vld1q_f64(pr.add(6)));
            let mut i0 = vuzp1q_f64(vld1q_f64(pi), vld1q_f64(pi.add(2)));
            let mut i1 = vuzp1q_f64(vld1q_f64(pi.add(4)), vld1q_f64(pi.add(6)));
            for (s, sv) in sc.iter().enumerate() {
                let sb = base.add(4 * s * stride + step_t * t);
                let h = 2 * step_t;
                let d = dup(sv);
                (r0, i0) = step(
                    d,
                    load(sb),
                    load(sb.add(stride)),
                    load(sb.add(2 * stride)),
                    load(sb.add(3 * stride)),
                    r0,
                    i0,
                );
                (r1, i1) = step(
                    d,
                    load(sb.add(h)),
                    load(sb.add(stride + h)),
                    load(sb.add(2 * stride + h)),
                    load(sb.add(3 * stride + h)),
                    r1,
                    i1,
                );
            }
            let (dr0, di0) = mulw(r0, i0, w);
            let (dr1, di1) = mulw(r1, i1, w);
            vst1q_f64(pr, vzip1q_f64(r0, dr0));
            vst1q_f64(pr.add(2), vzip2q_f64(r0, dr0));
            vst1q_f64(pr.add(4), vzip1q_f64(r1, dr1));
            vst1q_f64(pr.add(6), vzip2q_f64(r1, dr1));
            vst1q_f64(pi, vzip1q_f64(i0, di0));
            vst1q_f64(pi.add(2), vzip2q_f64(i0, di0));
            vst1q_f64(pi.add(4), vzip1q_f64(i1, di1));
            vst1q_f64(pi.add(6), vzip2q_f64(i1, di1));
            t += 4;
        }
        full
    }
}
