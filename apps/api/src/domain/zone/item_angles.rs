//! Exact certified-angle bounds (D21). Generated dyadic intervals require 192-bit
//! numerators; Q stat arithmetic remains checked i128. No interval is rounded to Q.

use std::cmp::Ordering;

use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

use super::ItemError;

/// Last nonzero movement direction, reduced to a primitive integer pair. New lives
/// face east. Target selection never modifies this value (design A10/D21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Direction", into = "Direction")]
pub struct FacingVector {
    x: i64,
    y: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Direction {
    x: i64,
    y: i64,
}

impl FacingVector {
    /// Initial facing for a new life.
    pub const EAST: Self = Self { x: 1, y: 0 };

    /// Reduce an actual displacement. A zero displacement leaves facing unchanged.
    #[must_use]
    pub fn moved(self, from: super::Vec2Fixed, to: super::Vec2Fixed) -> Self {
        let x = i64::from(to.x.raw()).saturating_sub(i64::from(from.x.raw()));
        let y = i64::from(to.y.raw()).saturating_sub(i64::from(from.y.raw()));
        let divisor = gcd(x.unsigned_abs(), y.unsigned_abs());
        let Ok(divisor) = i64::try_from(divisor) else {
            return self;
        };
        match (x.checked_div(divisor), y.checked_div(divisor)) {
            (Some(x), Some(y)) => Self { x, y },
            _ => self,
        }
    }

    /// Canonical signed components, independent of any eight-way client presentation.
    pub const fn components(self) -> (i64, i64) {
        (self.x, self.y)
    }

    /// Inclusive source cone comparison using the certified squared-cosine interval.
    /// An undecidable interval is corruption, never an approximate facing result.
    pub fn within(
        self,
        dx: i64,
        dy: i64,
        angle: &super::item_rules::CertifiedAngle,
        bound: u32,
    ) -> Result<bool, ItemError> {
        if !angle.certified
            || angle.degrees > 360
            || [self.x, self.y, dx, dy]
                .iter()
                .any(|n| n.unsigned_abs() > u64::from(bound))
        {
            return Err(ItemError("uncertified angular input"));
        }
        if dx == 0 && dy == 0 {
            return Ok(true);
        }
        let (x, y, dx, dy) =
            (i128::from(self.x), i128::from(self.y), i128::from(dx), i128::from(dy));
        let overflow = ItemError("angular comparison overflow");
        let dot = x
            .checked_mul(dx)
            .and_then(|v| y.checked_mul(dy).and_then(|w| v.checked_add(w)))
            .ok_or(overflow)?;
        if dot < 0 && angle.degrees < 180 {
            return Ok(false);
        }
        if dot >= 0 && angle.degrees >= 180 {
            return Ok(true);
        }
        let norm = |a: i128, b: i128| {
            a.checked_mul(a)
                .and_then(|v| b.checked_mul(b).and_then(|w| v.checked_add(w)))
        };
        let denominator = norm(x, y)
            .and_then(|v| norm(dx, dy).and_then(|w| v.checked_mul(w)))
            .ok_or(overflow)?;
        let numerator = dot.checked_mul(dot).ok_or(overflow)?;
        let lower = angle
            .cos2_lower
            .compare_integer_ratio(numerator, denominator)?;
        let upper = angle
            .cos2_upper
            .compare_integer_ratio(numerator, denominator)?;
        if angle.cos2_lower == angle.cos2_upper {
            return Ok(if dot >= 0 {
                !lower.is_gt()
            } else {
                !upper.is_lt()
            });
        }
        match (lower, upper, dot >= 0) {
            (_, Ordering::Less | Ordering::Equal, true)
            | (Ordering::Greater | Ordering::Equal, _, false) => Ok(true),
            (Ordering::Greater, _, true) | (_, Ordering::Less, false) => Ok(false),
            _ => Err(ItemError("angle certificate cannot decide bounded vector")),
        }
    }
}

fn gcd(mut x: u64, mut y: u64) -> u64 {
    while let Some(remainder) = x.checked_rem(y) {
        x = y;
        y = remainder;
    }
    x
}

impl TryFrom<Direction> for FacingVector {
    type Error = ItemError;
    fn try_from(value: Direction) -> Result<Self, Self::Error> {
        if gcd(value.x.unsigned_abs(), value.y.unsigned_abs()) != 1 {
            return Err(ItemError("nonprimitive facing"));
        }
        Ok(Self {
            x: value.x,
            y: value.y,
        })
    }
}
impl From<FacingVector> for Direction {
    fn from(value: FacingVector) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

/// A canonical exact cosine or squared-cosine bound in [-1,1]. The arbitrary-width
/// integers are necessary to retain the generator's certification for bounded vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AngleRatio {
    numerator: BigInt,
    denominator: BigInt,
}

// BigInt operations cannot overflow; divisors below are checked positive.
#[allow(clippy::arithmetic_side_effects)]
impl AngleRatio {
    /// Compare bounds exactly, including negative cosines at angles over 180 degrees.
    pub fn compare(&self, other: &Self) -> Ordering {
        (&self.numerator * &other.denominator).cmp(&(&other.numerator * &self.denominator))
    }

    /// Compare an exact bounded-vector ratio to the certificate without overflowing
    /// the cross product or turning a boundary equality into a nearby angle.
    pub fn compare_integer_ratio(
        &self,
        numerator: i128,
        denominator: i128,
    ) -> Result<Ordering, ItemError> {
        if denominator <= 0 {
            return Err(ItemError("nonpositive angular ratio denominator"));
        }
        Ok((&self.numerator * BigInt::from(denominator))
            .cmp(&(BigInt::from(numerator) * &self.denominator)))
    }
}
// Euclidean division only uses nonzero divisors and arbitrary-width integers.
#[allow(clippy::arithmetic_side_effects)]
impl TryFrom<String> for AngleRatio {
    type Error = ItemError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let invalid = ItemError("noncanonical certified angle ratio");
        let (numerator, denominator) = value.split_once('/').ok_or(invalid)?;
        let numerator: BigInt = numerator.parse().map_err(|_| invalid)?;
        let denominator: BigInt = denominator.parse().map_err(|_| invalid)?;
        if denominator <= BigInt::from(0) || numerator > denominator || numerator < -&denominator {
            return Err(invalid);
        }
        let mut left = if numerator < BigInt::from(0) {
            -&numerator
        } else {
            numerator.clone()
        };
        let mut right = denominator.clone();
        while right != BigInt::from(0) {
            let remainder = &left % &right;
            left = right;
            right = remainder;
        }
        let result = Self {
            numerator: numerator / &left,
            denominator: denominator / left,
        };
        if String::from(result.clone()) != value {
            return Err(invalid);
        }
        Ok(result)
    }
}
impl From<AngleRatio> for String {
    fn from(value: AngleRatio) -> Self {
        format!("{}/{}", value.numerator, value.denominator)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// T51/D157: an actual displacement reduces to its primitive gcd-normalized vector for
    /// axis, diagonal and general directions and large steps; zero keeps the facing.
    #[test]
    fn movement_facing_is_the_primitive_displacement() {
        let pos = |x, y| super::super::Vec2Fixed {
            x: super::super::Fixed::from_raw(x),
            y: super::super::Fixed::from_raw(y),
        };
        let start = FacingVector::EAST;
        for ((dx, dy), expected) in [
            ((500, 0), (1, 0)),
            ((-750, 0), (-1, 0)),
            ((0, 1250), (0, 1)),
            ((0, -3), (0, -1)),
            ((40, 40), (1, 1)),
            ((-25, 25), (-1, 1)),
            ((-9, -9), (-1, -1)),
            ((600, 400), (3, 2)),
            ((-1_000_000, 999_999), (-1_000_000, 999_999)),
            ((2_000_000_000, 1_000_000_000), (2, 1)),
        ] {
            let moved = start.moved(pos(100, 200), pos(100 + dx, 200 + dy));
            assert_eq!(moved.components(), expected, "{dx},{dy}");
            // A zero displacement (blocked or no-op step) keeps the previous facing.
            assert_eq!(moved.moved(pos(7, 7), pos(7, 7)), moved);
        }
    }

    #[test]
    fn movement_facing_and_inclusive_cones_preserve_exact_boundaries() {
        let pos = |x, y| super::super::Vec2Fixed {
            x: super::super::Fixed::from_raw(x),
            y: super::super::Fixed::from_raw(y),
        };
        let north_east = FacingVector::EAST.moved(pos(5, 8), pos(11, 14));
        assert_eq!(north_east.components(), (1, 1));
        assert_eq!(north_east.moved(pos(1, 2), pos(1, 2)), north_east);
        assert!(serde_json::from_str::<FacingVector>(r#"{"x":2,"y":2}"#).is_err());
        assert!(serde_json::from_str::<FacingVector>(r#"{"x":0,"y":0}"#).is_err());
        let angle = |degrees, squared: &str| super::super::item_rules::CertifiedAngle {
            degrees,
            special: "test-exact".into(),
            certified: true,
            blockers: vec![],
            cos_lower: AngleRatio::try_from("-1/1".to_owned()).unwrap(),
            cos_upper: AngleRatio::try_from("1/1".to_owned()).unwrap(),
            cos2_lower: AngleRatio::try_from(squared.to_owned()).unwrap(),
            cos2_upper: AngleRatio::try_from(squared.to_owned()).unwrap(),
        };
        let cone90 = angle(90, "1/2");
        for (x, y, expected) in [
            (1, 1, true),
            (1, -1, true),
            (1, 2, false),
            (-1, 1, false),
            (0, 0, true),
        ] {
            assert_eq!(FacingVector::EAST.within(x, y, &cone90, 256_000).unwrap(), expected);
        }
        let cone270 = angle(270, "1/2");
        assert!(FacingVector::EAST.within(-1, 1, &cone270, 256_000).unwrap());
        assert!(!FacingVector::EAST.within(-2, 1, &cone270, 256_000).unwrap());
        assert!(FacingVector::EAST
            .within(256_001, 0, &cone90, 256_000)
            .is_err());
        let mut ambiguous = cone90;
        ambiguous.cos2_lower = AngleRatio::try_from("1/4".to_owned()).unwrap();
        ambiguous.cos2_upper = AngleRatio::try_from("3/4".to_owned()).unwrap();
        assert!(FacingVector::EAST
            .within(1, 1, &ambiguous, 256_000)
            .is_err());
    }

    #[test]
    fn certificate_width_and_exact_special_boundaries_are_retained() {
        let bound = AngleRatio::try_from("6276862722586712153087596715245754440999340187840221120151/6277101735386680763835789423207666416102355444464034512896".to_owned()).unwrap();
        assert_eq!(bound.compare_integer_ratio(1, 1).unwrap(), Ordering::Less);
        assert_eq!(bound.compare_integer_ratio(0, 1).unwrap(), Ordering::Greater);
        let quarter = AngleRatio::try_from("1/4".to_owned()).unwrap();
        assert_eq!(quarter.compare_integer_ratio(25, 100).unwrap(), Ordering::Equal);
        assert_eq!(quarter.compare_integer_ratio(24, 100).unwrap(), Ordering::Greater);
        assert_eq!(quarter.compare_integer_ratio(26, 100).unwrap(), Ordering::Less);
        assert!(quarter.compare_integer_ratio(1, 0).is_err());
        let bytes = serde_json::to_vec(&bound).unwrap();
        assert_eq!(serde_json::from_slice::<AngleRatio>(&bytes).unwrap(), bound);
        for invalid in ["2/8", "1/0", "0/2", "+1/4", "1/-4", "2/1"] {
            assert!(AngleRatio::try_from(invalid.to_owned()).is_err());
        }
    }
}
