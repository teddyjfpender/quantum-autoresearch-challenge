//! Exact binary rationals `num / 2^shift`: every pinned float64 is one, and sums, differences
//! and products with integers and powers of two stay in the set, so the sparse spec and its
//! lane map never round.
use crate::spec::Exact;
use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

/// `num / 2^shift`, reduced (`num` odd unless `shift` is 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dyadic {
    pub num: BigInt,
    pub shift: u32,
}

impl Dyadic {
    #[must_use]
    pub fn zero() -> Self {
        Self {
            num: BigInt::zero(),
            shift: 0,
        }
    }

    #[must_use]
    pub fn new(num: BigInt, shift: u32) -> Self {
        Self { num, shift }.reduced()
    }

    /// The exact value of a finite float64.
    ///
    /// # Panics
    /// On NaN or infinity (never present in a loaded spec: `payload` rejects them).
    #[must_use]
    pub fn from_f64(v: f64) -> Self {
        assert!(v.is_finite(), "non-finite float in exact arithmetic");
        if v == 0.0 {
            return Self::zero();
        }
        let bits = v.to_bits();
        let exp = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
        let frac = bits & ((1u64 << 52) - 1);
        let (mant, e) = if exp == 0 {
            (frac, -1074)
        } else {
            (frac | (1u64 << 52), exp - 1075)
        };
        let num = BigInt::from(mant) * if v < 0.0 { -1 } else { 1 };
        if e >= 0 {
            Self::new(num << usize::try_from(e).unwrap_or(0), 0)
        } else {
            Self::new(num, u32::try_from(-e).unwrap_or(0))
        }
    }

    fn reduced(mut self) -> Self {
        if self.num.is_zero() {
            self.shift = 0;
            return self;
        }
        let tz = u32::try_from(self.num.trailing_zeros().unwrap_or(0)).unwrap_or(0);
        let t = tz.min(self.shift);
        self.num >>= t;
        self.shift -= t;
        self
    }

    fn at(&self, shift: u32) -> BigInt {
        &self.num << (shift - self.shift)
    }

    #[must_use]
    pub fn add(&self, o: &Self) -> Self {
        let s = self.shift.max(o.shift);
        Self::new(self.at(s) + o.at(s), s)
    }

    #[must_use]
    pub fn sub(&self, o: &Self) -> Self {
        let s = self.shift.max(o.shift);
        Self::new(self.at(s) - o.at(s), s)
    }

    #[must_use]
    pub fn abs(&self) -> Self {
        Self {
            num: self.num.abs(),
            shift: self.shift,
        }
    }

    /// `self * n / 2^d`, exactly.
    #[must_use]
    pub fn scale(&self, n: &BigInt, d: u32) -> Self {
        Self::new(&self.num * n, self.shift + d)
    }

    #[must_use]
    pub fn to_exact(&self) -> Exact {
        Exact {
            num: self.num.clone(),
            den: BigUint::from(1u8) << self.shift,
        }
    }

    /// Parses an exact `"num/den"` string whose denominator is a power of two.
    ///
    /// # Errors
    /// A malformed string or a denominator that is not a power of two.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let num: BigInt = n
            .trim()
            .parse()
            .map_err(|_| format!("bad numerator in {s:?}"))?;
        let den: BigUint = d
            .trim()
            .parse()
            .map_err(|_| format!("bad denominator in {s:?}"))?;
        let shift = den.bits().saturating_sub(1);
        if den.is_zero() || den != BigUint::from(1u8) << shift {
            return Err(format!("denominator of {s:?} is not a power of two"));
        }
        Ok(Self::new(
            num,
            u32::try_from(shift).map_err(|_| "denominator too large")?,
        ))
    }

    /// Nearest-ish double, for reports only.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        let (num, shift) = (&self.num, self.shift);
        let excess = num.bits().saturating_sub(60);
        let top = (num >> excess).to_f64().unwrap_or(f64::NAN);
        let e = i32::try_from(i64::try_from(excess).unwrap_or(0) - i64::from(shift)).unwrap_or(0);
        top * 2f64.powi(e)
    }

    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.num.sign() == Sign::Minus
    }
}

impl PartialOrd for Dyadic {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Dyadic {
    fn cmp(&self, o: &Self) -> Ordering {
        let s = self.shift.max(o.shift);
        self.at(s).cmp(&o.at(s))
    }
}

/// Exact sum of many dyadics, aligned once to the largest shift.
#[must_use]
pub fn sum<'a>(items: impl Iterator<Item = &'a Dyadic> + Clone) -> Dyadic {
    let s = items.clone().map(|d| d.shift).max().unwrap_or(0);
    Dyadic::new(items.map(|d| d.at(s)).sum(), s)
}
