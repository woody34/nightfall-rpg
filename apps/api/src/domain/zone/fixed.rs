//! Fixed-point positions and integer-only movement math (plan D7).
//!
//! One [`Fixed`] unit is 1/1000 tile. An `i32` covers about +/-2.1 million tiles, far beyond any
//! zone, so a position never needs a wider type; intermediate products that could exceed `i32`
//! are computed in `i128`/`u128` (exact over the whole `i32` range) and converted back with an
//! explicit overflow policy.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Fixed-point units per tile.
pub const UNITS_PER_TILE: i32 = 1000;

/// A coordinate or length in 1/1000 tile. Arithmetic is checked or saturating, never wrapping.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Fixed(i32);

impl Fixed {
    /// Zero.
    pub const ZERO: Self = Self(0);
    /// Largest representable value.
    pub const MAX: Self = Self(i32::MAX);
    /// Smallest representable value.
    pub const MIN: Self = Self(i32::MIN);

    /// Wraps a raw value in 1/1000 tile.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// Whole tiles. Saturates at `Fixed::MIN` / `Fixed::MAX` for out-of-range input.
    #[must_use]
    pub const fn from_tiles(tiles: i32) -> Self {
        Self(tiles.saturating_mul(UNITS_PER_TILE))
    }

    /// The raw value in 1/1000 tile. Conversion to wire floats happens in the interface layer
    /// (`interface::zone_mapping`) so this module never touches a float.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// `None` on overflow.
    #[must_use]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Clamps to `Fixed::MIN` / `Fixed::MAX` on overflow.
    #[must_use]
    pub const fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    /// Clamps to `Fixed::MIN` / `Fixed::MAX` on overflow.
    #[must_use]
    pub const fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    /// Multiplies by an integer factor. `None` on overflow.
    #[must_use]
    pub const fn checked_mul_int(self, factor: i32) -> Option<Self> {
        match self.0.checked_mul(factor) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Multiplies by an integer factor, clamping on overflow.
    #[must_use]
    pub const fn saturating_mul_int(self, factor: i32) -> Self {
        Self(self.0.saturating_mul(factor))
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Integer formatting only: "-12.345". `rem_euclid`/`div_euclid` would misplace the sign
        // for negatives, so split on the absolute value.
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let per_tile = UNITS_PER_TILE.unsigned_abs();
        let whole = abs.checked_div(per_tile).unwrap_or(0);
        let frac = abs.checked_rem(per_tile).unwrap_or(0);
        write!(f, "{sign}{whole}.{frac:03}")
    }
}

/// Movement speed in 1/1000 tile per tick. A newtype so it cannot be confused with a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Speed(u32);

impl Speed {
    /// Default walking pace for players and NPCs: 5 tiles/s at 10 ticks/s.
    pub const DEFAULT: Self = Self(500);

    /// 1/1000 tile per tick.
    #[must_use]
    pub const fn from_milli_tiles_per_tick(v: u32) -> Self {
        Self(v)
    }

    /// 1/1000 tile per tick.
    #[must_use]
    pub const fn milli_tiles_per_tick(self) -> u32 {
        self.0
    }
}

impl Default for Speed {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A point in the zone plane.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Vec2Fixed {
    /// East-west.
    pub x: Fixed,
    /// North-south.
    pub y: Fixed,
}

impl Vec2Fixed {
    /// Builds a point from two coordinates.
    #[must_use]
    pub const fn new(x: Fixed, y: Fixed) -> Self {
        Self { x, y }
    }

    /// A point at whole-tile coordinates (saturating).
    #[must_use]
    pub const fn from_tiles(x: i32, y: i32) -> Self {
        Self::new(Fixed::from_tiles(x), Fixed::from_tiles(y))
    }

    /// Squared Euclidean distance in (1/1000 tile)^2, exact for any two points: each axis
    /// delta fits `u32`, so the sum of squares fits `u128` with room to spare. Integer only,
    /// so comparisons are identical on every platform.
    #[must_use]
    pub fn distance_sq(self, other: Self) -> u128 {
        let dx = u128::from(self.x.0.abs_diff(other.x.0));
        let dy = u128::from(self.y.0.abs_diff(other.y.0));
        // Cannot overflow: each square < 2^64, the sum < 2^65.
        dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
    }

    /// `true` when `other` is at most `max` away. Integer-only: compares squared distances.
    #[must_use]
    pub fn within(self, other: Self, max: Fixed) -> bool {
        let m = u128::from(max.0.unsigned_abs());
        self.distance_sq(other) <= m.saturating_mul(m)
    }

    /// One movement step of at most `speed` toward `dest`.
    ///
    /// Rounding rule: when `dest` is within one step the result is `dest` itself (exact
    /// arrival, no overshoot, no stopping short). Otherwise each axis moves
    /// `trunc(delta * speed / ceil(|d|))`, where `|d|` is the integer square root of the exact
    /// squared distance rounded up; truncation is toward zero. Rounding `|d|` up and the
    /// quotient toward zero means the step is never longer than `speed` and never passes
    /// `dest` on either axis. If both axes truncate to zero (speeds of 1-2 units per tick on a
    /// diagonal), the step moves one unit along the axis with the larger remaining delta (x on
    /// a tie), so the distance strictly decreases and arrival is guaranteed. All intermediates
    /// are `i128`/`u128`, exact over the whole `i32` coordinate range.
    #[must_use]
    pub fn step_toward(self, dest: Self, speed: Speed) -> Self {
        let s = u128::from(speed.0);
        if s == 0 {
            return self;
        }
        let d_sq = self.distance_sq(dest);
        if d_sq <= s.saturating_mul(s) {
            return dest;
        }
        let dx = i128::from(dest.x.0).saturating_sub(i128::from(self.x.0));
        let dy = i128::from(dest.y.0).saturating_sub(i128::from(self.y.0));
        let len = i128::try_from(ceil_sqrt(d_sq)).unwrap_or(i128::MAX);
        let s = i128::from(speed.0);
        // len > s >= 1 here, so the divisions are defined and |step| <= s < |d|.
        let mut step_x = dx.saturating_mul(s).checked_div(len).unwrap_or(0);
        let mut step_y = dy.saturating_mul(s).checked_div(len).unwrap_or(0);
        if step_x == 0 && step_y == 0 {
            if dx.abs() >= dy.abs() {
                step_x = dx.signum();
            } else {
                step_y = dy.signum();
            }
        }
        Self::new(offset(self.x, step_x), offset(self.y, step_y))
    }
}

/// Smallest `r` with `r * r >= n`.
fn ceil_sqrt(n: u128) -> u128 {
    let r = n.isqrt();
    if r.saturating_mul(r) < n {
        r.saturating_add(1)
    } else {
        r
    }
}

/// `base + delta`, where the caller guarantees the result lies between `base` and a valid
/// `Fixed`; clamps to the `i32` range defensively instead of wrapping.
fn offset(base: Fixed, delta: i128) -> Fixed {
    let v = i128::from(base.0).saturating_add(delta);
    Fixed(i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX }))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::many_single_char_names
)]
mod tests {
    use super::*;

    #[test]
    fn from_tiles_scales_by_one_thousand() {
        assert_eq!(Fixed::from_tiles(3).raw(), 3000);
        assert_eq!(Fixed::from_tiles(-3).raw(), -3000);
        assert_eq!(Fixed::from_tiles(0), Fixed::ZERO);
    }

    #[test]
    fn from_tiles_saturates_at_the_i32_range() {
        // 2_147_483 tiles is the largest whole-tile value that fits.
        assert_eq!(Fixed::from_tiles(2_147_483).raw(), 2_147_483_000);
        assert_eq!(Fixed::from_tiles(2_147_484), Fixed::MAX);
        assert_eq!(Fixed::from_tiles(-2_147_484), Fixed::MIN);
        assert_eq!(Fixed::from_tiles(i32::MAX), Fixed::MAX);
        assert_eq!(Fixed::from_tiles(i32::MIN), Fixed::MIN);
    }

    #[test]
    fn checked_add_and_sub_report_overflow_at_the_boundaries() {
        let one = Fixed::from_raw(1);
        assert_eq!(Fixed::MAX.checked_sub(one), Some(Fixed::from_raw(i32::MAX - 1)));
        assert_eq!(Fixed::MAX.checked_add(Fixed::ZERO), Some(Fixed::MAX));
        assert_eq!(Fixed::MAX.checked_add(one), None);
        assert_eq!(Fixed::MIN.checked_sub(one), None);
        assert_eq!(Fixed::MIN.checked_add(one), Some(Fixed::from_raw(i32::MIN + 1)));
    }

    #[test]
    fn saturating_ops_clamp_instead_of_wrapping() {
        let one = Fixed::from_raw(1);
        assert_eq!(Fixed::MAX.saturating_add(one), Fixed::MAX);
        assert_eq!(Fixed::MIN.saturating_sub(one), Fixed::MIN);
        assert_eq!(Fixed::MIN.saturating_add(Fixed::MAX), Fixed::from_raw(-1));
        assert_eq!(Fixed::MAX.saturating_mul_int(2), Fixed::MAX);
        assert_eq!(Fixed::MIN.saturating_mul_int(2), Fixed::MIN);
        assert_eq!(Fixed::MAX.checked_mul_int(2), None);
        assert_eq!(Fixed::from_raw(7).checked_mul_int(-3), Some(Fixed::from_raw(-21)));
    }

    #[test]
    fn display_is_tiles_with_three_decimals() {
        assert_eq!(Fixed::from_raw(12_345).to_string(), "12.345");
        assert_eq!(Fixed::from_raw(-12_345).to_string(), "-12.345");
        assert_eq!(Fixed::from_raw(-5).to_string(), "-0.005");
        assert_eq!(Fixed::MIN.to_string(), "-2147483.648");
    }

    #[test]
    fn distance_sq_is_exact_across_the_whole_i32_range() {
        let a = Vec2Fixed::new(Fixed::MIN, Fixed::ZERO);
        let b = Vec2Fixed::new(Fixed::MAX, Fixed::ZERO);
        let d = u128::from(u32::MAX);
        assert_eq!(a.distance_sq(b), d * d);
        let c = Vec2Fixed::new(Fixed::MIN, Fixed::MIN);
        let e = Vec2Fixed::new(Fixed::MAX, Fixed::MAX);
        assert_eq!(c.distance_sq(e), 2 * d * d);
    }

    #[test]
    fn within_includes_the_exact_limit() {
        let o = Vec2Fixed::default();
        assert!(o.within(Vec2Fixed::from_tiles(3, 4), Fixed::from_tiles(5)));
        assert!(!o.within(
            Vec2Fixed::new(Fixed::from_tiles(3), Fixed::from_raw(4001)),
            Fixed::from_tiles(5)
        ));
    }

    #[test]
    fn step_arrives_exactly_when_within_one_step() {
        let o = Vec2Fixed::default();
        let dest = Vec2Fixed::new(Fixed::from_raw(300), Fixed::from_raw(400));
        assert_eq!(o.step_toward(dest, Speed::from_milli_tiles_per_tick(500)), dest);
        assert_eq!(
            o.step_toward(dest, Speed::from_milli_tiles_per_tick(499)),
            Vec2Fixed::new(Fixed::from_raw(299), Fixed::from_raw(399))
        );
    }

    #[test]
    fn zero_speed_never_moves() {
        let o = Vec2Fixed::default();
        assert_eq!(
            o.step_toward(Vec2Fixed::from_tiles(1, 1), Speed::from_milli_tiles_per_tick(0)),
            o
        );
    }

    #[test]
    fn speed_one_still_makes_progress_on_a_diagonal() {
        let o = Vec2Fixed::default();
        let dest = Vec2Fixed::new(Fixed::from_raw(10), Fixed::from_raw(10));
        let next = o.step_toward(dest, Speed::from_milli_tiles_per_tick(1));
        assert!(next.distance_sq(dest) < o.distance_sq(dest));
    }

    #[test]
    fn steps_near_the_i32_edges_do_not_overflow() {
        let a = Vec2Fixed::new(Fixed::MIN, Fixed::MIN);
        let b = Vec2Fixed::new(Fixed::MAX, Fixed::MAX);
        let next = a.step_toward(b, Speed::from_milli_tiles_per_tick(u32::MAX));
        assert!(next.distance_sq(b) < a.distance_sq(b));
        assert_eq!(b.step_toward(b, Speed::DEFAULT), b);
    }
}
