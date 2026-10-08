//! Fixed-point arithmetic for stats (plan docs/plans/phase-1-kill-a-monster.md §3.1).
//!
//! [`Scaled`] is a decimal with six fractional digits stored as an `i64` count of
//! `1 / Q`. Bonuses, coefficients and fractional derived stats (P.Atk, P.Def, accuracy,
//! evasion, attack speed) are `Scaled`; whole quantities (base stats, levels, live HP/MP,
//! damage, hate, XP) stay plain integers. Every product is formed in `i128`/`u128` with
//! checked operations and divided once at the end, with the rounding the plan names:
//! [`floor_div`] (mathematical floor, also for negative numerators) or [`ceil_div`].

use std::fmt;

/// Scale factor: one whole unit is `Q` raw units.
pub const Q: i64 = 1_000_000;

/// `Q` widened for intermediate products.
pub(crate) const Q128: i128 = 1_000_000;

/// A decimal in units of `1 / Q`. Ordering and equality are exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Scaled(i64);

impl Scaled {
    /// Zero.
    pub const ZERO: Self = Self(0);
    /// One whole unit.
    pub const ONE: Self = Self(Q);

    /// Wraps a raw count of `1 / Q`.
    #[must_use]
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// A whole number of units. `Overflow` past `i64::MAX / Q`.
    pub const fn from_int(units: i64) -> Result<Self, StatError> {
        match units.checked_mul(Q) {
            Some(raw) => Ok(Self(raw)),
            None => Err(StatError::Overflow),
        }
    }

    /// The raw count of `1 / Q`.
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Mathematical floor to whole units (`-0.5` floors to `-1`).
    #[must_use]
    pub const fn floor_units(self) -> i64 {
        self.0.div_euclid(Q)
    }

    /// Converts a checked `i128` intermediate back, failing rather than truncating.
    pub(crate) fn from_i128(raw: i128) -> Result<Self, StatError> {
        i64::try_from(raw)
            .map(Self)
            .map_err(|_| StatError::Overflow)
    }
}

impl fmt::Display for Scaled {
    /// Exact decimal form with six fractional digits, for logs and test messages only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let q = Q.unsigned_abs();
        let whole = abs.checked_div(q).unwrap_or(0);
        let frac = abs.checked_rem(q).unwrap_or(0);
        write!(f, "{sign}{whole}.{frac:06}")
    }
}

/// Why a stat calculation or a rule table was refused. No calculation wraps or saturates
/// silently: an input outside the proven range is an error the caller must handle.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatError {
    /// An intermediate or result does not fit its integer type.
    #[error("stat arithmetic overflowed")]
    Overflow,
    /// A divisor (defence, speed, a table denominator) was zero or negative.
    #[error("stat arithmetic divided by a non-positive value")]
    NonPositiveDivisor,
    /// A base stat outside the bonus table's index range.
    #[error("base stat {0} is outside the bonus table")]
    StatOutOfRange(u32),
    /// A level outside `1..=max_level` (or the sentinel where stated).
    #[error("level {0} is outside the rule tables")]
    LevelOutOfRange(u32),
    /// A random draw outside the range the formula allows.
    #[error("random draw {0} is outside the formula's range")]
    DrawOutOfRange(i64),
}

/// `a * b` in `i128`, `Overflow` instead of wrapping.
pub(crate) fn mul(a: i128, b: i128) -> Result<i128, StatError> {
    a.checked_mul(b).ok_or(StatError::Overflow)
}

/// `a + b` in `i128`, `Overflow` instead of wrapping.
pub(crate) fn add(a: i128, b: i128) -> Result<i128, StatError> {
    a.checked_add(b).ok_or(StatError::Overflow)
}

/// `a - b` in `i128`, `Overflow` instead of wrapping.
pub(crate) fn sub(a: i128, b: i128) -> Result<i128, StatError> {
    a.checked_sub(b).ok_or(StatError::Overflow)
}

/// `floor(n / d)` for `d > 0`, including negative `n` (Rust's `/` truncates toward zero).
pub fn floor_div(n: i128, d: i128) -> Result<i128, StatError> {
    if d <= 0 {
        return Err(StatError::NonPositiveDivisor);
    }
    n.checked_div_euclid(d).ok_or(StatError::Overflow)
}

/// `ceil(n / d)` for `d > 0`.
pub fn ceil_div(n: i128, d: i128) -> Result<i128, StatError> {
    let neg = n.checked_neg().ok_or(StatError::Overflow)?;
    floor_div(neg, d)?.checked_neg().ok_or(StatError::Overflow)
}

/// Floor integer square root.
#[must_use]
pub fn isqrt(n: u128) -> u128 {
    n.isqrt()
}

/// Narrows an `i128` result to a target integer type, `Overflow` if it does not fit.
pub(crate) fn narrow<T: TryFrom<i128>>(v: i128) -> Result<T, StatError> {
    T::try_from(v).map_err(|_| StatError::Overflow)
}
