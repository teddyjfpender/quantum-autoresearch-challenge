//! Vacuum expectation of a product of Majorana vectors, by Wick's theorem.
//!
//! `<vac| gamma(y_1) ... gamma(y_m) |vac> = Pf(A)`, `A_ij = <vac| gamma(y_i) gamma(y_j) |vac>`
//! for `i < j`. With `gamma_{2p} gamma_{2p+1} = i Z_p` and `Z_p |vac> = |vac>`:
//! even-even and odd-odd contractions are dot products, even-odd is `i` times the dot product and
//! odd-even is `-i` times it.
use super::Tv;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct C64 {
    pub re: f64,
    pub im: f64,
}

impl C64 {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };
    const ONE: Self = Self { re: 1.0, im: 0.0 };

    fn mul(self, o: Self) -> Self {
        Self {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }

    fn div(self, o: Self) -> Self {
        let d = o.re * o.re + o.im * o.im;
        Self {
            re: (self.re * o.re + self.im * o.im) / d,
            im: (self.im * o.re - self.re * o.im) / d,
        }
    }

    fn sub(self, o: Self) -> Self {
        Self {
            re: self.re - o.re,
            im: self.im - o.im,
        }
    }

    fn norm(self) -> f64 {
        self.re.hypot(self.im)
    }

    /// `self * w^k` with `w = exp(i pi / 4)`.
    #[must_use]
    pub fn scale_w(self, k: u8) -> Self {
        let th = f64::from(k % 8) * std::f64::consts::FRAC_PI_4;
        self.mul(Self {
            re: th.cos(),
            im: th.sin(),
        })
    }

    /// The `j` in `0..8` whose `w^j` is nearest, and the distance to it.
    #[must_use]
    pub fn nearest_w(self) -> (u8, f64) {
        (0..8u8)
            .map(|j| (j, self.sub(Self::ONE.scale_w(j)).norm()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, f64::INFINITY))
    }
}

fn contraction(a: &Tv, b: &Tv) -> C64 {
    let d: f64 = a.v.iter().zip(&b.v).map(|(x, y)| x * y).sum();
    match (a.odd, b.odd) {
        (false, true) => C64 { re: 0.0, im: d },
        (true, false) => C64 { re: 0.0, im: -d },
        _ => C64 { re: d, im: 0.0 },
    }
}

/// `<vac| gamma(y_1) ... gamma(y_m) |vac>`.
#[must_use]
pub fn vacuum_expectation(ys: &[&Tv]) -> C64 {
    let m = ys.len();
    if m % 2 == 1 {
        return C64::ZERO;
    }
    let mut a = vec![C64::ZERO; m * m];
    for i in 0..m {
        for j in i + 1..m {
            let c = contraction(ys[i], ys[j]);
            a[i * m + j] = c;
            a[j * m + i] = C64 {
                re: -c.re,
                im: -c.im,
            };
        }
    }
    pfaffian(&mut a, m)
}

/// Pfaffian of an antisymmetric `m x m` matrix (row-major, destroyed), by Gaussian elimination
/// with pivoting: congruences with unit-determinant matrices keep the Pfaffian, a swap of a row
/// and column pair negates it.
fn pfaffian(a: &mut [C64], m: usize) -> C64 {
    let mut pf = C64::ONE;
    for k in (0..m).step_by(2) {
        let piv = (k + 1..m)
            .max_by(|&x, &y| a[k * m + x].norm().total_cmp(&a[k * m + y].norm()))
            .unwrap_or(k + 1);
        if piv != k + 1 {
            swap(a, m, k + 1, piv);
            pf = C64 {
                re: -pf.re,
                im: -pf.im,
            };
        }
        let head = a[k * m + k + 1];
        if head.norm() == 0.0 {
            return C64::ZERO;
        }
        pf = pf.mul(head);
        let tau: Vec<C64> = (0..m).map(|j| a[k * m + j].div(head)).collect();
        let row: Vec<C64> = (0..m).map(|j| a[(k + 1) * m + j]).collect();
        for i in k + 2..m {
            for j in k + 2..m {
                // A_ij -= tau_j A_{i,k+1} + tau_i A_{k+1,j}, with A_{i,k+1} = -A_{k+1,i}.
                let d = tau[j].mul(row[i]).sub(tau[i].mul(row[j]));
                let v = &mut a[i * m + j];
                v.re += d.re;
                v.im += d.im;
            }
        }
    }
    pf
}

fn swap(a: &mut [C64], m: usize, x: usize, y: usize) {
    for j in 0..m {
        a.swap(x * m + j, y * m + j);
    }
    for i in 0..m {
        a.swap(i * m + x, i * m + y);
    }
}
