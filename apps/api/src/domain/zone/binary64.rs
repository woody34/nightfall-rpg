//! Exact IEEE-754 emulation for the Phase 5 economy (design §6.2): `round_p` over exact
//! rationals gives `fl` (binary64) and `fl32` (binary32) with round-to-nearest-ties-to-even,
//! failing closed outside the normal range. No floating-point arithmetic is involved; a
//! binary64 value is carried as its raw bit pattern and converted exactly.
//!
//! `fl(decimal)` is Java `Double.parseDouble` (correctly rounded, E30); `fl32(fl(x))` is the
//! `float f = (float) double` narrowing (E37); the exact power rounded once is E32.

#![allow(clippy::arithmetic_side_effects)] // BigUint arithmetic cannot overflow.

use num_bigint::BigUint;

/// Fail-closed domain error: a value outside the normal binary range or a malformed literal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("binary64 emulation: {0}")]
pub struct Binary64Error(pub String);

fn zero() -> BigUint {
    BigUint::from(0_u8)
}

fn one() -> BigUint {
    BigUint::from(1_u8)
}

/// An exact rational `sign · num / den` with `den > 0`. Not reduced: every operation here is
/// exact, so reduction is never needed for correctness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rational {
    negative: bool,
    num: BigUint,
    den: BigUint,
}

impl Rational {
    /// Exact integer.
    #[must_use]
    pub fn integer(value: i64) -> Self {
        Self {
            negative: value < 0,
            num: BigUint::from(value.unsigned_abs()),
            den: one(),
        }
    }

    /// Exact `num / den` with `den > 0`.
    pub fn new(negative: bool, num: BigUint, den: BigUint) -> Result<Self, Binary64Error> {
        if den == zero() {
            return Err(Binary64Error("zero denominator".into()));
        }
        Ok(Self { negative, num, den })
    }

    /// A plain decimal literal `-?digits(.digits)?`, exactly. No exponent, no sign other than
    /// a leading minus, no whitespace, no empty parts.
    pub fn parse_decimal(text: &str) -> Result<Self, Binary64Error> {
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (int_part, frac_part) = body.split_once('.').map_or((body, ""), |(i, f)| (i, f));
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if !digits(int_part) || (body.contains('.') && !digits(frac_part)) {
            return Err(Binary64Error(format!("{text:?} is not a plain decimal")));
        }
        let mut num = zero();
        for b in int_part.bytes().chain(frac_part.bytes()) {
            num = num * BigUint::from(10_u8) + BigUint::from(b - b'0');
        }
        let den = BigUint::from(10_u8).pow(
            u32::try_from(frac_part.len())
                .map_err(|_| Binary64Error("decimal literal too long".into()))?,
        );
        Ok(Self { negative, num, den })
    }

    /// True for an exact zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.num == zero()
    }

    /// Exact product.
    #[must_use]
    pub fn mul(&self, rhs: &Self) -> Self {
        Self {
            negative: self.negative != rhs.negative,
            num: &self.num * &rhs.num,
            den: &self.den * &rhs.den,
        }
    }

    /// Exact quotient; fails on a zero divisor.
    pub fn div(&self, rhs: &Self) -> Result<Self, Binary64Error> {
        if rhs.is_zero() {
            return Err(Binary64Error("division by zero".into()));
        }
        Ok(Self {
            negative: self.negative != rhs.negative,
            num: &self.num * &rhs.den,
            den: &self.den * &rhs.num,
        })
    }

    /// Exact sum.
    #[must_use]
    pub fn add(&self, rhs: &Self) -> Self {
        let left = &self.num * &rhs.den;
        let right = &rhs.num * &self.den;
        let den = &self.den * &rhs.den;
        if self.negative == rhs.negative {
            return Self {
                negative: self.negative,
                num: left + right,
                den,
            };
        }
        if left >= right {
            Self {
                negative: self.negative,
                num: left - right,
                den,
            }
        } else {
            Self {
                negative: rhs.negative,
                num: right - left,
                den,
            }
        }
    }

    /// Exact difference.
    #[must_use]
    pub fn sub(&self, rhs: &Self) -> Self {
        self.add(&Self {
            negative: !rhs.negative,
            num: rhs.num.clone(),
            den: rhs.den.clone(),
        })
    }

    /// Exact integer power `self^n` (negative `n` inverts; fails for `0^negative`).
    pub fn pow(&self, n: i32) -> Result<Self, Binary64Error> {
        let magnitude = n.unsigned_abs();
        let (num, den) = (self.num.pow(magnitude), self.den.pow(magnitude));
        let negative = self.negative && magnitude % 2 == 1;
        if n < 0 {
            if self.is_zero() {
                return Err(Binary64Error("zero to a negative power".into()));
            }
            return Ok(Self {
                negative,
                num: den,
                den: num,
            });
        }
        Ok(Self { negative, num, den })
    }

    /// `⌊self⌋` for a non-negative value, as `u64`; fails on a negative value or overflow.
    pub fn floor_u64(&self) -> Result<u64, Binary64Error> {
        if self.negative && !self.is_zero() {
            return Err(Binary64Error("floor of a negative value".into()));
        }
        u64::try_from(&self.num / &self.den)
            .map_err(|_| Binary64Error("floor does not fit u64".into()))
    }

    /// Ordering of two exact rationals.
    #[must_use]
    pub fn compare(&self, rhs: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self.negative && !self.is_zero(), rhs.negative && !rhs.is_zero()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (negative, _) => {
                let left = &self.num * &rhs.den;
                let right = &rhs.num * &self.den;
                if negative {
                    right.cmp(&left)
                } else {
                    left.cmp(&right)
                }
            },
        }
    }

    /// `round_p` of design §6.2: round-to-nearest-ties-to-even to `p` significant bits,
    /// failing closed outside `[emin, emax]` (subnormals and overflow are invariant errors).
    fn round_p(&self, p: u32, emin: i64, emax: i64) -> Result<Rounded, Binary64Error> {
        if self.is_zero() {
            return Ok(Rounded {
                negative: self.negative,
                mantissa: zero(),
                exponent: 0,
                p,
            });
        }
        let (num0, den0) = (&self.num, &self.den);
        let mut exp = i64::try_from(num0.bits())
            .ok()
            .zip(i64::try_from(den0.bits()).ok())
            .map(|(a, b)| a - b)
            .ok_or_else(|| Binary64Error("operand too wide".into()))?;
        // Correct ⌊log2 x⌋ by one comparison: 2^e ≤ x < 2^(e+1).
        let shifted =
            |value: &BigUint, by: i64| -> BigUint { value << usize::try_from(by).unwrap_or(0) };
        if shifted(num0, (0_i64.saturating_sub(exp)).max(0)) < shifted(den0, exp.max(0)) {
            exp -= 1;
        }
        if exp < emin || exp > emax {
            return Err(Binary64Error(format!("exponent {exp} outside the normal range")));
        }
        let shift = i64::from(p) - 1 - exp;
        let (num, den) = if shift >= 0 {
            (shifted(num0, shift), den0.clone())
        } else {
            (num0.clone(), shifted(den0, -shift))
        };
        let mut quotient = &num / &den;
        let remainder = &num % &den;
        let twice = &remainder << 1_usize;
        if twice > den || (twice == den && (&quotient & one()) == one()) {
            quotient += one();
        }
        let mut exponent = exp;
        if quotient == (one() << usize::try_from(p).unwrap_or(0)) {
            quotient >>= 1_usize;
            exponent += 1;
            if exponent > emax {
                return Err(Binary64Error("overflow after rounding".into()));
            }
        }
        Ok(Rounded {
            negative: self.negative,
            mantissa: quotient,
            exponent,
            p,
        })
    }

    /// `fl(x)`: binary64 (`p = 53`, `emin = −1022`, `emax = 1023`).
    pub fn fl(&self) -> Result<Binary64, Binary64Error> {
        Binary64::from_rounded(&self.round_p(53, -1022, 1023)?)
    }

    /// `fl32(x)`: binary32 (`p = 24`, `emin = −126`, `emax = 127`), returned as the exact
    /// binary64 it widens to (a `float` → `double` widening is exact).
    pub fn fl32(&self) -> Result<Binary64, Binary64Error> {
        let rounded = self.round_p(24, -126, 127)?;
        // Widening: the same value with 29 more mantissa bits.
        Binary64::from_rounded(&Rounded {
            negative: rounded.negative,
            mantissa: if rounded.mantissa == zero() {
                zero()
            } else {
                &rounded.mantissa << 29_usize
            },
            exponent: rounded.exponent,
            p: 53,
        })
    }
}

/// A rounded value `sign · mantissa · 2^(exponent − p + 1)` with `mantissa ∈ [2^(p−1), 2^p)`.
struct Rounded {
    negative: bool,
    mantissa: BigUint,
    exponent: i64,
    p: u32,
}

/// A binary64 value as its raw IEEE bit pattern (sign, 11-bit biased exponent, 52-bit
/// fraction). Only finite normal values and signed zero are ever constructed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Binary64(pub u64);

impl Binary64 {
    /// Positive zero.
    pub const ZERO: Self = Self(0);
    /// Exactly one.
    pub const ONE: Self = Self(0x3ff0_0000_0000_0000);

    fn from_rounded(rounded: &Rounded) -> Result<Self, Binary64Error> {
        let sign = if rounded.negative { 1_u64 << 63 } else { 0 };
        if rounded.mantissa == zero() {
            return Ok(Self(sign));
        }
        if rounded.p != 53 {
            return Err(Binary64Error("not a binary64 mantissa".into()));
        }
        let mantissa = u64::try_from(&rounded.mantissa)
            .map_err(|_| Binary64Error("mantissa too wide".into()))?;
        let biased = u64::try_from(rounded.exponent + 1023)
            .map_err(|_| Binary64Error("exponent outside binary64".into()))?;
        if !(1..=2046).contains(&biased) || mantissa >> 52 != 1 {
            return Err(Binary64Error("not a normal binary64".into()));
        }
        Ok(Self(sign | (biased << 52) | (mantissa & ((1 << 52) - 1))))
    }

    /// Parse the 16-hex-digit raw-bits form the generated tables use.
    pub fn parse_hex(text: &str) -> Result<Self, Binary64Error> {
        if text.len() != 16
            || !text
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Binary64Error(format!("{text:?} is not 16 lowercase hex digits")));
        }
        u64::from_str_radix(text, 16)
            .map(Self)
            .map_err(|_| Binary64Error(format!("{text:?} is not hex")))
    }

    /// The 16-hex-digit raw-bits form.
    #[must_use]
    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }

    /// The exact value; fails closed for NaN, infinities and subnormals.
    pub fn rational(self) -> Result<Rational, Binary64Error> {
        let negative = self.0 >> 63 == 1;
        let biased = (self.0 >> 52) & 0x7ff;
        let fraction = self.0 & ((1 << 52) - 1);
        if biased == 0 {
            if fraction != 0 {
                return Err(Binary64Error("subnormal binary64".into()));
            }
            return Ok(Rational {
                negative,
                num: zero(),
                den: one(),
            });
        }
        if biased == 0x7ff {
            return Err(Binary64Error("non-finite binary64".into()));
        }
        let mantissa = BigUint::from(fraction | (1 << 52));
        let exponent =
            i64::try_from(biased).map_err(|_| Binary64Error("exponent".into()))? - 1023 - 52;
        let (num, den) = if exponent >= 0 {
            (mantissa << usize::try_from(exponent).unwrap_or(0), one())
        } else {
            (mantissa, one() << usize::try_from(-exponent).unwrap_or(0))
        };
        Ok(Rational { negative, num, den })
    }
}

/// `fl(decimal)`: Java `Double.parseDouble` of a plain decimal literal (E30).
pub fn fl_decimal(text: &str) -> Result<Binary64, Binary64Error> {
    Rational::parse_decimal(text)?.fl()
}

/// `fl32(decimal)`: Java `Float.parseFloat` of a plain decimal literal, widened.
pub fn fl32_decimal(text: &str) -> Result<Binary64, Binary64Error> {
    Rational::parse_decimal(text)?.fl32()
}

/// `fl32(fl(rate × G))` (E37): the double product of two binary64 values narrowed to float.
pub fn g_fold(rate: Binary64, g: Binary64) -> Result<Binary64, Binary64Error> {
    rate.rational()?
        .mul(&g.rational()?)
        .fl()?
        .rational()?
        .fl32()
}

/// `pow56(k) = fl((5/6 as binary64)^k)`, `k ∈ [1, 79]` (E32); fails closed outside.
pub fn pow56(k: i32) -> Result<Binary64, Binary64Error> {
    if !(1..=79).contains(&k) {
        return Err(Binary64Error(format!("pow56 k={k} fails closed")));
    }
    Rational::integer(5)
        .div(&Rational::integer(6))?
        .fl()?
        .rational()?
        .pow(k)?
        .fl()
}

/// `pow13(d) = fl(fl(1.3)^d)`, `d ∈ [−120, 120]` (E32); fails closed outside.
pub fn pow13(d: i32) -> Result<Binary64, Binary64Error> {
    if !(-120..=120).contains(&d) {
        return Err(Binary64Error(format!("pow13 d={d} fails closed")));
    }
    fl_decimal("1.3")?.rational()?.pow(d)?.fl()
}

/// The vendor unit price of design §6.2: `⌊fl(P · fl(1 + fl(fl(tax)/100 + fl(castle/100))))⌋`.
pub fn vendor_unit_price(
    reference_price: u64,
    tax_literal: &str,
    castle_percent: u32,
) -> Result<u64, Binary64Error> {
    let hundred = Rational::integer(100);
    let t = fl_decimal(tax_literal)?.rational()?.div(&hundred)?.fl()?;
    let c = Rational::integer(i64::from(castle_percent))
        .div(&hundred)?
        .fl()?;
    let s = t.rational()?.add(&c.rational()?).fl()?;
    let u = Rational::integer(1).add(&s.rational()?).fl()?;
    let price = i64::try_from(reference_price)
        .map_err(|_| Binary64Error("reference price too large".into()))?;
    if reference_price >= 1 << 53 {
        return Err(Binary64Error("reference price is not exactly representable".into()));
    }
    Rational::integer(price)
        .mul(&u.rational()?)
        .fl()?
        .rational()?
        .floor_u64()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn decimal_parsing_is_correctly_rounded_and_plain_only() {
        assert_eq!(fl_decimal("1").unwrap(), Binary64::ONE);
        assert_eq!(fl_decimal("0.1").unwrap().hex(), "3fb999999999999a");
        assert_eq!(fl_decimal("1.3").unwrap().hex(), "3ff4cccccccccccd");
        assert_eq!(fl_decimal("0.82").unwrap().hex(), "3fea3d70a3d70a3d");
        assert_eq!(fl_decimal("188.85499572753906").unwrap().hex(), "40679b5c20000000");
        assert_eq!(fl_decimal("0.0003000000142492354").unwrap().hex(), "3f33a92a40000000");
        assert_eq!(fl_decimal("100").unwrap().hex(), "4059000000000000");
        assert_eq!(fl_decimal("0").unwrap(), Binary64::ZERO);
        assert_eq!(fl_decimal("-0.5").unwrap().hex(), "bfe0000000000000");
        for bad in ["1e3", "+1", " 1", "1.", ".5", "", "1,5", "--1", "0x1"] {
            assert!(fl_decimal(bad).is_err(), "{bad:?}");
        }
        // Round trip through the exact rational and back.
        let value = fl_decimal("0.10000000000000009").unwrap();
        assert_eq!(value.hex(), "3fb99999999999a0");
        assert_eq!(value.rational().unwrap().fl().unwrap(), value);
    }

    #[test]
    fn narrowing_double_rounds_as_java_does() {
        // DR5 counterexample: fl32(fl(G)) = 1 while a single rounding would give 1 + 2^-23.
        let g = Rational::new(
            false,
            BigUint::from(2_164_260_992_u64),
            BigUint::from(2_164_260_863_u64),
        )
        .unwrap();
        assert_eq!(g.fl().unwrap().rational().unwrap().fl32().unwrap(), Binary64::ONE);
        assert_ne!(g.fl32().unwrap(), Binary64::ONE);
        assert_eq!(fl32_decimal("1").unwrap(), Binary64::ONE);
        assert_eq!(fl32_decimal("0.1").unwrap().hex(), "3fb99999a0000000");
        assert_eq!(g_fold(Binary64::ONE, Binary64::ONE).unwrap(), Binary64::ONE);
        assert_eq!(
            g_fold(Binary64::ONE, fl_decimal("1.5").unwrap())
                .unwrap()
                .hex(),
            "3ff8000000000000"
        );
    }

    #[test]
    fn vendor_prices_follow_the_java_double_chain() {
        assert_eq!(vendor_unit_price(100, "15", 0).unwrap(), 114);
        assert_eq!(vendor_unit_price(100, "15", 0).unwrap(), 114);
        assert_eq!(vendor_unit_price(590, "15", 0).unwrap(), 678);
        assert_eq!(vendor_unit_price(590, "15", 5).unwrap(), 708);
        assert_eq!(vendor_unit_price(105_000, "10", 0).unwrap(), 115_500);
        assert_eq!(vendor_unit_price(105_000, "15", 0).unwrap(), 120_749);
        assert_eq!(vendor_unit_price(0, "0", 0).unwrap(), 0);
        assert!(vendor_unit_price(1 << 53, "0", 0).is_err());
    }

    #[test]
    fn powers_are_exact_rational_powers_rounded_once() {
        assert_eq!(pow56(1).unwrap().hex(), "3feaaaaaaaaaaaab");
        assert_eq!(pow56(2).unwrap().hex(), "3fe638e38e38e38f");
        assert_eq!(pow56(11).unwrap().hex(), "3fc13a2dda98e2c7");
        assert_eq!(pow13(0).unwrap(), Binary64::ONE);
        assert_eq!(pow13(1).unwrap().hex(), "3ff4cccccccccccd");
        assert!(pow56(0).is_err() && pow56(80).is_err());
        assert!(pow13(121).is_err() && pow13(-121).is_err());
        assert!(pow13(-120).is_ok() && pow13(120).is_ok());
    }

    #[test]
    fn out_of_range_values_fail_closed() {
        let tiny = Rational::new(false, one(), one() << 1100_usize).unwrap();
        assert!(tiny.fl().is_err());
        let huge = Rational::new(false, one() << 1100_usize, one()).unwrap();
        assert!(huge.fl().is_err());
        let under_float = Rational::new(false, one(), one() << 130_usize).unwrap();
        assert!(under_float.fl32().is_err());
        assert!(Binary64(0x7ff0_0000_0000_0000).rational().is_err());
        assert!(Binary64(1).rational().is_err());
        assert!(Binary64::parse_hex("3FF0000000000000").is_err());
        assert!(Rational::integer(1).div(&Rational::integer(0)).is_err());
        // The raid clamp intermediate: fl(d × fl(0.15)) + 1 at d = −6 is 0.10000000000000009.
        let step = Rational::integer(-6).mul(&fl_decimal("0.15").unwrap().rational().unwrap());
        let value = step
            .fl()
            .unwrap()
            .rational()
            .unwrap()
            .add(&Rational::integer(1));
        assert_eq!(value.fl().unwrap().hex(), "3fb99999999999a0");
    }
}
