//! `U256` wide arithmetic for the E15 chance test (design §2.3, §6.2): `success iff
//! r · (100·den) < num · 2^64`, evaluated exactly. Every operand here is an unsigned 64-bit
//! limb product; no 128-bit chance product exists in the Phase 5 engine (a source scan
//! enforces it), and no operation can overflow silently: widths beyond 256 bits fail closed.

use super::binary64::Binary64;

/// A 256-bit unsigned integer, little-endian limbs (ordering compares the most significant
/// limb first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct U256([u64; 4]);

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.iter().rev().cmp(other.0.iter().rev())
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Fail-closed width error: a Phase 5 chance product wider than 255 bits (D-W bounds 141 at
/// `G = 1`, 173 at `G = 2^-32`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("U256 overflow: {0}")]
pub struct WideError(pub &'static str);

impl U256 {
    /// Zero.
    pub const ZERO: Self = Self([0; 4]);

    /// From a 64-bit value.
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self([value, 0, 0, 0])
    }

    /// The limbs, least significant first.
    #[must_use]
    pub const fn limbs(self) -> [u64; 4] {
        self.0
    }

    /// Significant bits (0 for zero).
    #[must_use]
    pub fn bits(self) -> u32 {
        let mut index: usize = 4;
        while index > 0 {
            index = index.saturating_sub(1);
            let limb = self.0.get(index).copied().unwrap_or(0);
            if limb != 0 {
                let top = u32::try_from(index).unwrap_or(0).saturating_mul(64);
                return top.saturating_add(64_u32.saturating_sub(limb.leading_zeros()));
            }
        }
        0
    }

    /// `self · rhs` with `rhs` a 64-bit value; fails closed on overflow.
    pub fn checked_mul_u64(self, rhs: u64) -> Result<Self, WideError> {
        let mut out = [0_u64; 4];
        let mut carry: u64 = 0;
        for (index, limb) in self.0.iter().enumerate() {
            // (limb · rhs) + carry fits in 128 bits of exact integer arithmetic: the two 64-bit
            // halves are extracted without any wider chance value being formed.
            let (low, high) = mul_64(*limb, rhs);
            let (sum, overflow) = low.overflowing_add(carry);
            if let Some(slot) = out.get_mut(index) {
                *slot = sum;
            }
            carry = high.saturating_add(u64::from(overflow));
        }
        if carry != 0 {
            return Err(WideError("product wider than 256 bits"));
        }
        Ok(Self(out))
    }

    /// `self << shift`; fails closed when a set bit would leave the top limb.
    pub fn checked_shl(self, shift: u32) -> Result<Self, WideError> {
        if shift == 0 {
            return Ok(self);
        }
        if self.bits().saturating_add(shift) > 256 {
            return Err(WideError("shift wider than 256 bits"));
        }
        let limb_shift = usize::try_from(shift / 64).unwrap_or(4);
        let bit_shift = shift % 64;
        let mut out = [0_u64; 4];
        for (index, limb) in self.0.iter().enumerate() {
            let target = index.saturating_add(limb_shift);
            if let Some(slot) = out.get_mut(target) {
                *slot |= limb << bit_shift;
            }
            if bit_shift != 0 {
                if let Some(slot) = out.get_mut(target.saturating_add(1)) {
                    *slot |= limb >> 64_u32.wrapping_sub(bit_shift);
                }
            }
        }
        Ok(Self(out))
    }
}

/// Full 64×64→128 product as two 64-bit halves, by 32-bit schoolbook (no 128-bit integer).
fn mul_64(a: u64, b: u64) -> (u64, u64) {
    let (a0, a1) = (a & 0xffff_ffff, a >> 32);
    let (b0, b1) = (b & 0xffff_ffff, b >> 32);
    let p00 = a0.wrapping_mul(b0);
    let p01 = a0.wrapping_mul(b1);
    let p10 = a1.wrapping_mul(b0);
    let p11 = a1.wrapping_mul(b1);
    // Middle column: p01 + p10 + (p00 >> 32), accumulated in 64 bits with carry capture.
    let (middle, carry_a) = p01.overflowing_add(p10);
    let (middle, carry_b) = middle.overflowing_add(p00 >> 32);
    let carry = u64::from(carry_a).saturating_add(u64::from(carry_b));
    let low = (middle << 32) | (p00 & 0xffff_ffff);
    let high = p11.wrapping_add(middle >> 32).wrapping_add(carry << 32);
    (low, high)
}

/// `chance(num, den)` of design §2.3 for an integer percent ratio: success iff
/// `r · (100·den) < num · 2^64`.
pub fn chance_ratio(num: u64, den: u64, draw: u64) -> Result<bool, WideError> {
    if den == 0 {
        return Err(WideError("zero denominator"));
    }
    let left = U256::from_u64(draw)
        .checked_mul_u64(100)?
        .checked_mul_u64(den)?;
    let right = U256::from_u64(num).checked_shl(64)?;
    Ok(left < right)
}

/// `chance_bound(k, n)` of design §2.3: `Rnd.get(n) < k` under E15, i.e. `chance(k·100, n)`,
/// which reduces to `r · n < k · 2^64`.
pub fn chance_bound(k: u64, n: u64, draw: u64) -> Result<bool, WideError> {
    if n == 0 {
        return Err(WideError("zero bound"));
    }
    let left = U256::from_u64(draw).checked_mul_u64(n)?;
    let right = U256::from_u64(k).checked_shl(64)?;
    Ok(left < right)
}

/// The exact dyadic form of a non-negative binary64 chance `c = num / 2^den_log2`. Fails
/// closed for a negative, non-finite or subnormal chance. The width bound is enforced where
/// the terms are built: `r · 100 · 2^den_log2` must fit 256 bits (D-W: 141 bits at `G = 1`,
/// 173 at `G = 2^-32`, well inside).
pub fn dyadic_chance(chance: Binary64) -> Result<(U256, u32), WideError> {
    let bits = chance.0;
    if bits >> 63 == 1 && bits != 1 << 63 {
        return Err(WideError("negative chance"));
    }
    let biased = (bits >> 52) & 0x7ff;
    let fraction = bits & ((1 << 52) - 1);
    if biased == 0 {
        if fraction != 0 {
            return Err(WideError("subnormal chance"));
        }
        return Ok((U256::ZERO, 0));
    }
    if biased == 0x7ff {
        return Err(WideError("non-finite chance"));
    }
    let mantissa = fraction | (1 << 52);
    // value = mantissa · 2^(biased − 1075)
    let exponent = i64::try_from(biased).unwrap_or(0).saturating_sub(1075);
    if exponent >= 0 {
        let shift = u32::try_from(exponent).map_err(|_| WideError("exponent"))?;
        Ok((U256::from_u64(mantissa).checked_shl(shift)?, 0))
    } else {
        let shift = u32::try_from(exponent.unsigned_abs()).map_err(|_| WideError("exponent"))?;
        Ok((U256::from_u64(mantissa), shift))
    }
}

/// The two sides of the E15 comparison for a dyadic chance `num / 2^den_log2` in percent:
/// `(r · 100 · 2^den_log2, num · 2^64)`, both in `U256`; a width beyond 256 bits fails closed.
pub fn chance_terms(num: U256, den_log2: u32, draw: u64) -> Result<(U256, U256), WideError> {
    let left = U256::from_u64(draw)
        .checked_mul_u64(100)?
        .checked_shl(den_log2)?;
    let right = num.checked_shl(64)?;
    Ok((left, right))
}

/// `chance(C)` for a binary64 percent chance (drops, design §3.2): the pinned
/// `chance > nextDouble() · 100` with the double chance exact, at the 2^-64 draw resolution.
/// Exactly one draw per call is the caller's duty (rule P3); this is the comparison only.
pub fn chance_binary64(chance: Binary64, draw: u64) -> Result<bool, WideError> {
    let (num, den_log2) = dyadic_chance(chance)?;
    let (left, right) = chance_terms(num, den_log2, draw)?;
    Ok(left < right)
}

/// The number of draws `r ∈ [0, 2^64)` that succeed for a binary64 chance, capped at `2^64`
/// (the oracle's `dc_vectors.pass_threshold`): `⌈num · 2^64 / (100 · 2^den_log2)⌉`.
pub fn pass_count(chance: Binary64) -> Result<U256, WideError> {
    let (num, den_log2) = dyadic_chance(chance)?;
    let right = num.checked_shl(64)?;
    let divisor = U256::from_u64(100).checked_shl(den_log2)?;
    let (quotient, remainder) = div_rem(right, divisor)?;
    let count = if remainder == U256::ZERO {
        quotient
    } else {
        add_one(quotient)?
    };
    let all = U256::from_u64(1).checked_shl(64)?;
    Ok(if count > all { all } else { count })
}

fn add_one(value: U256) -> Result<U256, WideError> {
    let mut limbs = value.0;
    for limb in &mut limbs {
        let (sum, overflow) = limb.overflowing_add(1);
        *limb = sum;
        if !overflow {
            return Ok(U256(limbs));
        }
    }
    Err(WideError("increment wider than 256 bits"))
}

/// Long division by shift-subtract; the divisor is small and this runs only in tests and
/// evidence, never per draw.
fn div_rem(dividend: U256, divisor: U256) -> Result<(U256, U256), WideError> {
    if divisor == U256::ZERO {
        return Err(WideError("division by zero"));
    }
    let mut quotient = U256::ZERO;
    let mut remainder = U256::ZERO;
    let mut bit = dividend.bits();
    while bit > 0 {
        bit = bit.saturating_sub(1);
        remainder = remainder.checked_shl(1)?;
        let limb = usize::try_from(bit / 64).unwrap_or(0);
        if dividend
            .0
            .get(limb)
            .is_some_and(|l| (l >> (bit % 64)) & 1 == 1)
        {
            if let Some(slot) = remainder.0.get_mut(0) {
                *slot |= 1;
            }
        }
        if remainder >= divisor {
            remainder = sub(remainder, divisor);
            let mut set = quotient.0;
            if let Some(slot) = set.get_mut(limb) {
                *slot |= 1 << (bit % 64);
            }
            quotient = U256(set);
        }
    }
    Ok((quotient, remainder))
}

fn sub(a: U256, b: U256) -> U256 {
    let mut out = [0_u64; 4];
    let mut borrow = 0_u64;
    for index in 0..4 {
        let x = a.0.get(index).copied().unwrap_or(0);
        let y = b.0.get(index).copied().unwrap_or(0);
        let (d1, b1) = x.overflowing_sub(y);
        let (d2, b2) = d1.overflowing_sub(borrow);
        if let Some(slot) = out.get_mut(index) {
            *slot = d2;
        }
        borrow = u64::from(b1 || b2);
    }
    U256(out)
}

impl std::cmp::PartialOrd<u64> for U256 {
    fn partial_cmp(&self, other: &u64) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&Self::from_u64(*other)))
    }
}

impl std::cmp::PartialEq<u64> for U256 {
    fn eq(&self, other: &u64) -> bool {
        *self == Self::from_u64(*other)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::domain::zone::binary64::fl_decimal;

    #[test]
    fn wide_multiplication_and_shift_are_exact_and_fail_closed() {
        let (low, high) = mul_64(u64::MAX, u64::MAX);
        assert_eq!((low, high), (1, u64::MAX - 1));
        let (low, high) = mul_64(0x1234_5678_9abc_def0, 0x0fed_cba9_8765_4321);
        assert_eq!(low, 0x2236_d88f_e561_8cf0);
        assert_eq!(high, 0x0121_fa00_ad77_d742);
        let max = U256([u64::MAX; 4]);
        assert!(max.checked_mul_u64(2).is_err());
        assert!(max.checked_shl(1).is_err());
        assert_eq!(U256::from_u64(1).checked_shl(255).unwrap().bits(), 256);
        assert!(U256::from_u64(1).checked_shl(256).is_err());
        assert_eq!(U256::from_u64(0).bits(), 0);
        assert_eq!(U256::from_u64(1).checked_shl(64).unwrap(), U256([0, 1, 0, 0]));
        let (q, r) =
            div_rem(U256::from_u64(1).checked_shl(64).unwrap(), U256::from_u64(3)).unwrap();
        assert_eq!(q, U256::from_u64(6_148_914_691_236_517_205));
        assert_eq!(r, U256::from_u64(1));
    }

    #[test]
    fn chance_thresholds_reproduce_the_oracle_dc_vectors() {
        // economy-oracle-drops.json dc_vectors: C_bits → pass_threshold (draw count).
        for (bits, threshold) in [
            ("4058ffffffffffff", "18446744073709548995"),
            ("4059000000000000", "18446744073709551616"),
            ("4059000000000001", "18446744073709551616"),
            ("40679b5c20000000", "18446744073709551616"),
            ("3f33a92a40000000", "55340234849649"),
        ] {
            let chance = Binary64::parse_hex(bits).unwrap();
            let count = pass_count(chance).unwrap();
            let expected: u128 = threshold.parse().unwrap();
            let expected = if expected == 1 << 64 {
                U256::from_u64(1).checked_shl(64).unwrap()
            } else {
                U256::from_u64(u64::try_from(expected).unwrap())
            };
            assert_eq!(count, expected, "{bits}");
            // The last succeeding draw is threshold − 1, the first failing one is threshold.
            if let Some(last) = (expected.0[1] == 0).then_some(expected.0[0]) {
                assert!(chance_binary64(chance, last - 1).unwrap(), "{bits}");
                assert!(!chance_binary64(chance, last).unwrap(), "{bits}");
            } else {
                assert!(chance_binary64(chance, u64::MAX).unwrap(), "{bits}: every draw passes");
            }
        }
        // Zero chance never passes, 100 always, and the draw is still taken by the caller (P3).
        assert!(!chance_binary64(Binary64::ZERO, 0).unwrap());
        assert!(chance_binary64(fl_decimal("100").unwrap(), u64::MAX).unwrap());
        assert!(chance_ratio(100, 1, u64::MAX).unwrap());
        assert!(!chance_ratio(0, 1, 0).unwrap());
    }

    #[test]
    fn chance_bound_is_the_integer_test_at_draw_resolution() {
        // Rnd.nextInt(99) < 90 ⇔ r · 99 < 90 · 2^64.
        let edge = (U256::from_u64(90).checked_shl(64).unwrap(), 99_u64);
        let (q, r) = div_rem(edge.0, U256::from_u64(edge.1)).unwrap();
        let first_fail = if r == U256::ZERO { q.0[0] } else { q.0[0] + 1 };
        assert!(chance_bound(90, 99, first_fail - 1).unwrap());
        assert!(!chance_bound(90, 99, first_fail).unwrap());
        assert!(chance_bound(40, 100, 0).unwrap());
        assert!(!chance_bound(0, 100, 0).unwrap());
        assert!(chance_bound(100, 100, u64::MAX).unwrap());
        assert!(chance_bound(0, 0, 0).is_err());
    }

    #[test]
    fn dyadic_forms_fail_closed_outside_the_pinned_domain() {
        assert!(dyadic_chance(Binary64(0xbff0_0000_0000_0000)).is_err(), "negative");
        assert!(dyadic_chance(Binary64(0x7ff0_0000_0000_0000)).is_err(), "infinite");
        assert!(dyadic_chance(Binary64(1)).is_err(), "subnormal");
        // Widths beyond 256 bits fail closed at the comparison, not silently.
        assert!(chance_binary64(Binary64(0x0010_0000_0000_0000), u64::MAX).is_err(), "2^-1022");
        assert!(chance_binary64(Binary64(0x3c00_0000_0000_0000), u64::MAX).is_ok(), "2^-63 fits");
        assert_eq!(dyadic_chance(Binary64::ZERO).unwrap(), (U256::ZERO, 0));
        let (num, den) = dyadic_chance(fl_decimal("0.5").unwrap()).unwrap();
        assert_eq!((num, den), (U256::from_u64(1 << 52), 53));
        let (num, den) = dyadic_chance(fl_decimal("100").unwrap()).unwrap();
        assert_eq!((num, den), (U256::from_u64(0x19_0000_0000_0000), 46));
    }
}
