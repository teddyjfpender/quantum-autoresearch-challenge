//! Exact rationals for coefficients, normalizations and rounding errors.
//!
//! Every value the harness compares against a threshold (the lane map's rounding error, the
//! spec's lambda) is computed here without floating point. Pinned f64 integrals are exact binary
//! rationals, so `from_f64` introduces no rounding.
use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

/// Longest numerator or denominator `Exact::from_bytes` accepts, in bytes (1024 bits). The
/// encoding carries a lane map's `lambda_decl` in the untrusted `lanemap.bin`; the lane maps
/// cap its dyadic exponent at 512 bits, so an honest value is at most a few dozen bytes. An
/// uncapped part made the constructor's gcd quadratic in the file size (two 1 MB
/// parts took 53 s, and the time grows with the square) and every rounding-error term as long
/// as the file.
pub const MAX_PART_BYTES: usize = 128;

/// Exact rational `num / den`, `den > 0`. Constructors reduce to lowest terms, so the derived
/// equality is value equality for values built through them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exact {
    pub num: BigInt,
    pub den: BigUint,
}

impl Exact {
    /// `num / den` in lowest terms.
    ///
    /// # Panics
    /// If `den` is zero.
    #[must_use]
    pub fn new(num: BigInt, den: BigUint) -> Self {
        assert!(!den.is_zero(), "Exact: zero denominator");
        let g = BigInt::from(num.magnitude().gcd(&den));
        if g.is_one() || g.is_zero() {
            return Self { num, den };
        }
        let den = BigInt::from(den) / &g;
        Self {
            num: num / g,
            den: den.magnitude().clone(),
        }
    }

    /// Parses `"n/d"` or `"n"` (decimal integers, `d > 0`), as `Display` writes them.
    ///
    /// # Errors
    /// Anything else.
    pub fn parse(s: &str) -> Result<Self, String> {
        let bad = || format!("not an exact rational: {s:?}");
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let num: BigInt = n.trim().parse().map_err(|_| bad())?;
        let den: BigUint = d.trim().parse().map_err(|_| bad())?;
        if den.is_zero() {
            return Err(bad());
        }
        Ok(Self::new(num, den))
    }

    #[must_use]
    pub fn zero() -> Self {
        Self::from_int(0)
    }

    #[must_use]
    pub fn from_int(v: i64) -> Self {
        Self {
            num: BigInt::from(v),
            den: BigUint::one(),
        }
    }

    /// `m / 2^e`.
    #[must_use]
    pub fn dyadic(m: BigInt, e: u32) -> Self {
        Self::new(m, BigUint::one() << e)
    }

    /// The exact value of a finite f64 (every finite f64 is a binary rational).
    ///
    /// # Errors
    /// A NaN or infinite input.
    pub fn from_f64(v: f64) -> Result<Self, String> {
        if !v.is_finite() {
            return Err(format!("Exact::from_f64: non-finite value {v}"));
        }
        if v == 0.0 {
            return Ok(Self::zero());
        }
        let bits = v.to_bits();
        let sign = if bits >> 63 == 1 { -1 } else { 1 };
        let exp = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
        let frac = bits & ((1u64 << 52) - 1);
        let (mant, e) = if exp == 0 {
            (frac, -1074)
        } else {
            (frac | (1u64 << 52), exp - 1075)
        };
        let m = BigInt::from(mant) * sign;
        Ok(if e >= 0 {
            Self::new(m << e, BigUint::one())
        } else {
            let shift = u32::try_from(-e).unwrap_or(u32::MAX);
            Self::dyadic(m, shift)
        })
    }

    #[must_use]
    pub fn add(&self, o: &Self) -> Self {
        let num = &self.num * BigInt::from(o.den.clone()) + &o.num * BigInt::from(self.den.clone());
        Self::new(num, &self.den * &o.den)
    }

    #[must_use]
    pub fn sub(&self, o: &Self) -> Self {
        self.add(&o.neg())
    }

    #[must_use]
    pub fn mul(&self, o: &Self) -> Self {
        Self::new(&self.num * &o.num, &self.den * &o.den)
    }

    #[must_use]
    pub fn neg(&self) -> Self {
        Self {
            num: -self.num.clone(),
            den: self.den.clone(),
        }
    }

    #[must_use]
    pub fn abs(&self) -> Self {
        Self {
            num: self.num.abs(),
            den: self.den.clone(),
        }
    }

    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.num.sign() == Sign::Minus
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.num.is_zero()
    }

    /// The exponent `e` when the value is `m / 2^e` in lowest terms, else `None`.
    #[must_use]
    pub fn dyadic_exponent(&self) -> Option<u32> {
        let tz = self.den.trailing_zeros().unwrap_or(0);
        (self.den == BigUint::one() << tz).then(|| u32::try_from(tz).unwrap_or(u32::MAX))
    }

    /// Nearest-ish f64 for reporting only (never used in a pass/fail decision).
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        let nb = i64::try_from(self.num.bits()).unwrap_or(i64::MAX);
        let db = i64::try_from(self.den.bits()).unwrap_or(i64::MAX);
        // Scale both to about 64 significant bits so neither overflows f64.
        let ns = (nb - 64).max(0);
        let ds = (db - 64).max(0);
        let n = (&self.num >> ns).to_f64().unwrap_or(f64::NAN);
        let d = (&self.den >> ds).to_f64().unwrap_or(f64::NAN);
        let shift = i32::try_from(ns - ds).unwrap_or(0);
        n / d * 2f64.powi(shift)
    }

    /// Serializes as `(sign byte, u32 numerator length, LE numerator bytes, u32 denominator
    /// length, LE denominator bytes)`.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.is_negative())];
        for mag in [self.num.magnitude().to_bytes_le(), self.den.to_bytes_le()] {
            out.extend_from_slice(&u32::try_from(mag.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&mag);
        }
        out
    }

    /// Inverse of `to_bytes`; returns the value and the bytes consumed. Each part is at most
    /// [`MAX_PART_BYTES`] long.
    ///
    /// # Errors
    /// Truncated input, a part longer than [`MAX_PART_BYTES`], or a zero denominator.
    pub fn from_bytes(b: &[u8]) -> Result<(Self, usize), String> {
        let neg = *b.first().ok_or("Exact: truncated")? == 1;
        let mut at = 1;
        let mut part = || -> Result<BigUint, String> {
            let len = b.get(at..at + 4).ok_or("Exact: truncated")?;
            let len = u32::from_le_bytes(len.try_into().map_err(|_| "Exact: truncated")?);
            let len = usize::try_from(len).map_err(|e| e.to_string())?;
            if len > MAX_PART_BYTES {
                return Err(format!(
                    "Exact: a {len}-byte part is longer than {MAX_PART_BYTES} bytes"
                ));
            }
            let bytes = b.get(at + 4..at + 4 + len).ok_or("Exact: truncated")?;
            at += 4 + len;
            Ok(BigUint::from_bytes_le(bytes))
        };
        let mag = part()?;
        let den = part()?;
        if den.is_zero() {
            return Err("Exact: zero denominator".into());
        }
        let sign = if neg { Sign::Minus } else { Sign::Plus };
        Ok((Self::new(BigInt::from_biguint(sign, mag), den), at))
    }
}

impl PartialOrd for Exact {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Exact {
    fn cmp(&self, o: &Self) -> Ordering {
        let l = &self.num * BigInt::from(o.den.clone());
        let r = &o.num * BigInt::from(self.den.clone());
        l.cmp(&r)
    }
}

impl std::fmt::Display for Exact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_round_trips_exactly() {
        for v in [0.0, 1.0, -0.5, 3.5e-5, 1e-300, 5e-324, 1234.5678] {
            let e = Exact::from_f64(v).unwrap();
            assert_eq!(e.to_f64(), v, "{v}");
        }
        assert_eq!(
            Exact::from_f64(0.75).unwrap(),
            Exact::dyadic(BigInt::from(3), 2)
        );
    }

    #[test]
    fn arithmetic_and_bytes() {
        let a = Exact::new(BigInt::from(1), BigUint::from(3u8));
        let b = Exact::new(BigInt::from(-1), BigUint::from(6u8));
        assert_eq!(a.add(&b), Exact::new(BigInt::from(1), BigUint::from(6u8)));
        assert!(b < a && b.abs() < a);
        let (back, used) = Exact::from_bytes(&b.to_bytes()).unwrap();
        assert_eq!((back, used), (b.clone(), b.to_bytes().len()));
        assert_eq!(Exact::dyadic(BigInt::from(5), 3).dyadic_exponent(), Some(3));
        assert_eq!(a.dyadic_exponent(), None);
    }
}
