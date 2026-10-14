//! Checked arithmetic for Phase 3. Pinned source: l2j-game abfde049, paths below relative
//! to `src/main/java/com/l2jserver/gameserver/`. No caller here consumes the zone RNG.
//!
//! Keep exact immutable literals and binary32 intermediates wider than Q until the named
//! conversion boundary. Java integer casts truncate; `Math.round` instead breaks ties up.

use super::scaled::{add, ceil_div, floor_div, mul, narrow, round_div, sub};
use super::{StatError, Tick, Q};

/// A reduced exact rational used locally, never a second mutable resource representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactRatio {
    numerator: i128,
    denominator: i128,
}

impl serde::Serialize for ExactRatio {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("{}/{}", self.numerator, self.denominator))
    }
}

impl ExactRatio {
    /// Parse an exact signed decimal or fraction without a binary floating-point hop.
    /// Exponents, whitespace, empty components and unrepresentable values are rejected.
    pub fn parse_literal(value: &str) -> Result<Self, StatError> {
        let invalid = || StatError::Overflow;
        if value.is_empty() || value.trim() != value || value.starts_with('+') {
            return Err(invalid());
        }
        if let Some((numerator, denominator)) = value.split_once('/') {
            return Self::new(
                numerator.parse().map_err(|_| invalid())?,
                denominator.parse().map_err(|_| invalid())?,
            );
        }
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        let digits = whole.strip_prefix('-').unwrap_or(whole);
        if digits.is_empty()
            || !digits.bytes().all(|c| c.is_ascii_digit())
            || !fraction.bytes().all(|c| c.is_ascii_digit())
            || value.ends_with('.')
        {
            return Err(invalid());
        }
        let scale = 10_i128
            .checked_pow(u32::try_from(fraction.len()).map_err(|_| invalid())?)
            .ok_or_else(invalid)?;
        let whole: i128 = whole.parse().map_err(|_| invalid())?;
        let fraction: i128 = if fraction.is_empty() {
            0
        } else {
            fraction.parse().map_err(|_| invalid())?
        };
        let signed_fraction = if value.starts_with('-') {
            fraction.checked_neg().ok_or_else(invalid)?
        } else {
            fraction
        };
        Self::new(add(mul(whole, scale)?, signed_fraction)?, scale)
    }

    /// A whole quantity, without multiplication by Q.
    pub const fn whole(value: i64) -> Self {
        Self {
            numerator: value as i128,
            denominator: 1,
        }
    }

    /// Canonicalizes a fraction, requiring a positive denominator.
    pub fn new(numerator: i128, denominator: i128) -> Result<Self, StatError> {
        if denominator <= 0 {
            return Err(StatError::NonPositiveDivisor);
        }
        let magnitude = numerator.checked_abs().ok_or(StatError::Overflow)?;
        let divisor = gcd(magnitude, denominator);
        Ok(Self {
            numerator: numerator.checked_div(divisor).ok_or(StatError::Overflow)?,
            denominator: denominator
                .checked_div(divisor)
                .ok_or(StatError::Overflow)?,
        })
    }

    /// Exact normalized parts for provenance, tests and immutable-rule codecs.
    pub const fn parts(self) -> (i128, i128) {
        (self.numerator, self.denominator)
    }

    /// Checked sum, cross-canceling denominators before multiplication.
    pub fn checked_add(self, rhs: Self) -> Result<Self, StatError> {
        let common = gcd(self.denominator, rhs.denominator);
        let left = rhs
            .denominator
            .checked_div(common)
            .ok_or(StatError::Overflow)?;
        let right = self
            .denominator
            .checked_div(common)
            .ok_or(StatError::Overflow)?;
        Self::new(
            add(mul(self.numerator, left)?, mul(rhs.numerator, right)?)?,
            mul(self.denominator, left)?,
        )
    }

    /// Checked difference, including negative modifiers.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, StatError> {
        self.checked_add(Self::new(
            rhs.numerator.checked_neg().ok_or(StatError::Overflow)?,
            rhs.denominator,
        )?)
    }

    /// Checked product, cross-canceling before multiplication.
    pub fn checked_mul(self, rhs: Self) -> Result<Self, StatError> {
        let a = gcd(self.numerator.checked_abs().ok_or(StatError::Overflow)?, rhs.denominator);
        let b = gcd(rhs.numerator.checked_abs().ok_or(StatError::Overflow)?, self.denominator);
        Self::new(
            mul(
                self.numerator.checked_div(a).ok_or(StatError::Overflow)?,
                rhs.numerator.checked_div(b).ok_or(StatError::Overflow)?,
            )?,
            mul(
                self.denominator.checked_div(b).ok_or(StatError::Overflow)?,
                rhs.denominator.checked_div(a).ok_or(StatError::Overflow)?,
            )?,
        )
    }

    /// Checked division by a positive quantity (defence/speed/coefficient).
    pub fn checked_div(self, rhs: Self) -> Result<Self, StatError> {
        if rhs.numerator <= 0 {
            return Err(StatError::NonPositiveDivisor);
        }
        self.checked_mul(Self::new(rhs.denominator, rhs.numerator)?)
    }

    /// Exact comparison without conversion to Q.
    pub fn checked_cmp(self, rhs: Self) -> Result<std::cmp::Ordering, StatError> {
        let delta = self.checked_sub(rhs)?;
        Ok(delta.numerator.cmp(&0))
    }

    /// Source minimum/maximum clamps, performed before quantization.
    pub fn clamp(self, low: Self, high: Self) -> Result<Self, StatError> {
        if low.checked_cmp(high)?.is_gt() {
            return Err(StatError::Overflow);
        }
        Ok(if self.checked_cmp(low)?.is_lt() {
            low
        } else if self.checked_cmp(high)?.is_gt() {
            high
        } else {
            self
        })
    }

    /// Java `(int)`/`(long)` conversion, before a checked target-type narrowing.
    pub fn trunc(self) -> Result<i128, StatError> {
        self.numerator
            .checked_div(self.denominator)
            .ok_or(StatError::Overflow)
    }

    /// Mathematical floor, including negative modifiers.
    pub fn floor(self) -> Result<i128, StatError> {
        floor_div(self.numerator, self.denominator)
    }

    /// Mathematical ceiling, used for source duration and scheduler boundaries.
    pub fn ceil(self) -> Result<i128, StatError> {
        ceil_div(self.numerator, self.denominator)
    }

    /// Java nearest-integer rounding with ties toward positive infinity.
    pub fn round(self) -> Result<i128, StatError> {
        round_div(self.numerator, self.denominator)
    }

    /// Q conversion at a named runtime boundary; preserves six fractional digits.
    pub fn to_q(self) -> Result<i64, StatError> {
        narrow(self.checked_mul(Self::whole(Q))?.trunc()?)
    }

    /// Integer implementation of binary32 round-to-nearest, ties-to-even for normal values.
    /// Out-of-range source inputs are rejected at rule validation, never approximated.
    /// `model/effects/AbstractEffect.java:148-149` and `model/stats/Formulas.java:1301`.
    pub fn binary32(self) -> Result<Self, StatError> {
        self.binary(23, -126, 127)
    }

    /// Integer-only Java double rounding for source-mandated intermediate boundaries.
    /// Rejects values whose exact widened binary rational cannot fit checked i128.
    pub fn binary64(self) -> Result<Self, StatError> {
        self.binary(52, -1022, 1023)
    }

    fn binary(
        self,
        fraction_bits: u32,
        minimum_exponent: i32,
        maximum_exponent: i32,
    ) -> Result<Self, StatError> {
        if self.numerator == 0 {
            return Ok(self);
        }
        let negative = self.numerator < 0;
        let mut normalized =
            Self::new(self.numerator.checked_abs().ok_or(StatError::Overflow)?, self.denominator)?;
        let mut exponent = 0_i32;
        while normalized.checked_cmp(Self::whole(2))?.is_ge() {
            normalized = normalized.checked_div(Self::whole(2))?;
            exponent = exponent.checked_add(1).ok_or(StatError::Overflow)?;
        }
        while normalized.checked_cmp(Self::whole(1))?.is_lt() {
            normalized = normalized.checked_mul(Self::whole(2))?;
            exponent = exponent.checked_sub(1).ok_or(StatError::Overflow)?;
        }
        if !(minimum_exponent..=maximum_exponent).contains(&exponent) {
            return Err(StatError::Overflow);
        }
        let significand = normalized.checked_mul(Self::whole(
            1_i64
                .checked_shl(fraction_bits)
                .ok_or(StatError::Overflow)?,
        ))?;
        let floor = significand.floor()?;
        let remainder = significand.checked_sub(Self::new(floor, 1)?)?;
        let half = remainder.checked_cmp(Self::new(1, 2)?)?;
        let rounded = add(floor, i128::from(half.is_gt() || (half.is_eq() && floor & 1 == 1)))?;
        let shift = exponent
            .checked_sub(i32::try_from(fraction_bits).map_err(|_| StatError::Overflow)?)
            .ok_or(StatError::Overflow)?;
        let power = 2_i128
            .checked_pow(shift.unsigned_abs())
            .ok_or(StatError::Overflow)?;
        let signed = if negative {
            rounded.checked_neg().ok_or(StatError::Overflow)?
        } else {
            rounded
        };
        if shift >= 0 {
            Self::new(mul(signed, power)?, 1)
        } else {
            Self::new(signed, power)
        }
    }
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    // Both arguments are nonnegative; b begins positive. The remainder divisor is guarded.
    while b != 0 {
        let remainder = a.checked_rem(b).unwrap_or(0);
        a = b;
        b = remainder;
    }
    a
}

/// Original animation, including fractional milliseconds. Only the selected speed enters;
/// physical/magic selection is the caller's responsibility. `Formulas.java:1027-1046`.
/// Charged shots are exposed for oracle completeness; Phase 3 callers always supply false.
pub fn cast_animation(
    hit_time_ms: u32,
    speed: u32,
    is_static: bool,
    is_magic: bool,
    charged_shots: bool,
) -> Result<ExactRatio, StatError> {
    let base = ExactRatio::whole(i64::from(hit_time_ms));
    let mut animation = if is_static {
        base
    } else {
        base.checked_mul(ExactRatio::whole(333))?
            .checked_div(ExactRatio::whole(i64::from(speed)))?
    };
    if is_magic && charged_shots {
        animation = animation.checked_mul(ExactRatio::new(3, 5)?)?;
    }
    if hit_time_ms > 500 && animation.checked_cmp(ExactRatio::whole(500))?.is_lt() {
        animation = ExactRatio::whole(500);
    }
    Ok(animation)
}

/// Independently stored animation-derived deadlines (v3 F3/F4); cooling time is unused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CastDeadlines {
    /// Original animation truncated to source milliseconds.
    pub animation_ms: u64,
    /// Launch may occur at start, but cannot precede it.
    pub launch_at: Tick,
    /// Completion is always at least one tick after start.
    pub complete_at: Tick,
    /// Exclusive damage-interruption cutoff; start means an empty window.
    pub interrupt_until: Tick,
}

/// `L2Character.java:1508,1614-1645,4577-4581`. Clamp launch before the +400ms impact
/// stage; ceiling is applied independently to each absolute offset, never cumulatively.
pub fn cast_deadlines(
    start: Tick,
    animation: ExactRatio,
    continuous: bool,
) -> Result<CastDeadlines, StatError> {
    if animation.checked_cmp(ExactRatio::whole(0))?.is_lt() {
        return Err(StatError::Overflow);
    }
    let whole = animation.trunc()?;
    let effective = animation.checked_sub(ExactRatio::whole(if continuous { 300 } else { 0 }))?;
    let launch = sub(effective.trunc()?, 400)?.max(0);
    let impact = add(launch, if whole == 0 { 0 } else { 400 })?;
    let deadline = |offset: i128| -> Result<Tick, StatError> {
        start
            .0
            .checked_add(narrow(offset)?)
            .map(Tick)
            .ok_or(StatError::Overflow)
    };
    Ok(CastDeadlines {
        animation_ms: narrow(whole)?,
        launch_at: deadline(ceil_div(launch, 100)?)?,
        complete_at: deadline(ceil_div(impact, 100)?.max(1))?,
        interrupt_until: deadline(sub(floor_div(whole, 100)?, 2)?.max(0))?,
    })
}

/// Source target-count sampling, with draw presence enforced as well as draw range.
/// Width zero returns zero (unlimited), ignoring offset, with no draw. Positive width
/// consumes exactly one draw, including width one. `Skill.java:637-641`, `AffectScope.java:386`.
pub fn affect_limit(offset: u32, width: u32, roll: Option<u32>) -> Result<u32, StatError> {
    match (width, roll) {
        (0, None) => Ok(0),
        (0, Some(value)) => Err(StatError::DrawOutOfRange(i64::from(value))),
        (_, Some(value)) if value < width => offset.checked_add(value).ok_or(StatError::Overflow),
        (_, value) => Err(StatError::DrawOutOfRange(value.map_or(-1, i64::from))),
    }
}

/// Formulas.java:1648-1666: resistance modifies BUFF cancellation before its first
/// Java int cast. Preserve binary64 operation boundaries and saturating int conversion.
pub fn cancel_base_rate(
    rate: i32,
    vulnerability_q: i64,
    proficiency_q: i64,
) -> Result<i32, StatError> {
    let sum = ExactRatio::new(i128::from(vulnerability_q), i128::from(Q))?
        .checked_add(ExactRatio::new(i128::from(proficiency_q), i128::from(Q))?)?
        .binary64()?;
    let resistance = ExactRatio::whole(1)
        .checked_sub(sum.checked_div(ExactRatio::whole(100))?.binary64()?)?
        .binary64()?;
    if resistance.parts().0 == 0 {
        return Ok(match rate.cmp(&0) {
            std::cmp::Ordering::Less => i32::MIN,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => i32::MAX,
        });
    }
    let (numerator, denominator) = resistance.parts();
    let reciprocal = if numerator < 0 {
        ExactRatio::new(
            denominator.checked_neg().ok_or(StatError::Overflow)?,
            numerator.checked_neg().ok_or(StatError::Overflow)?,
        )?
    } else {
        ExactRatio::new(denominator, numerator)?
    };
    let value = ExactRatio::whole(i64::from(rate))
        .checked_mul(reciprocal)?
        .binary64()?
        .trunc()?;
    i32::try_from(value.clamp(i128::from(i32::MIN), i128::from(i32::MAX)))
        .map_err(|_| StatError::Overflow)
}

/// Formulas.java:1747-1750: level modifier has a second Java int cast, followed by
/// per-skill min/max and a strict roll comparison. DEBUFF cancellation instead uses <=.
pub fn cancel_buff_chance(
    rate: i32,
    caster_level: i64,
    effect_level: i64,
    min: u32,
    max: u32,
) -> Result<i64, StatError> {
    if min > max {
        return Err(StatError::Overflow);
    }
    let adjusted = if effect_level > 0 {
        let difference = caster_level
            .checked_sub(effect_level)
            .ok_or(StatError::Overflow)?;
        let factor = ExactRatio::whole(1)
            .checked_add(ExactRatio::new(i128::from(difference), 100)?.binary64()?)?
            .binary64()?;
        ExactRatio::whole(i64::from(rate))
            .checked_mul(factor)?
            .binary64()?
            .trunc()?
    } else {
        i128::from(rate)
    };
    let integer = adjusted.clamp(i128::from(i32::MIN), i128::from(i32::MAX));
    i64::try_from(integer.clamp(i128::from(min), i128::from(max))).map_err(|_| StatError::Overflow)
}

/// Ideal periodic interval in milliseconds and its exact binary32 multiplier, widened without
/// further rounding. `AbstractEffect.java:149`; character.properties:145 (666ms default).
pub fn periodic_interval(ticks: u32, ratio_ms: u32) -> Result<(u32, ExactRatio), StatError> {
    let interval = ticks
        .checked_mul(ratio_ms)
        .filter(|v| *v > 0 && i32::try_from(*v).is_ok())
        .ok_or(StatError::Overflow)?;
    // Java converts the numerator to binary32 before division, then rounds the quotient.
    let numerator = ExactRatio::whole(i64::from(interval)).binary32()?;
    let multiplier = numerator.checked_div(ExactRatio::whole(1000))?.binary32()?;
    Ok((interval, multiplier))
}

/// Pulse n is ceil(n*interval/100), not n*ceil(interval/100); n starts at one.
/// `BuffInfo.java:269-272` and v3 §1.3's tick adaptation.
pub fn periodic_deadline(
    applied: Tick,
    pulse_index: u64,
    interval_ms: u32,
) -> Result<Tick, StatError> {
    if pulse_index == 0 || interval_ms == 0 {
        return Err(StatError::Overflow);
    }
    let offset: u64 =
        narrow(ceil_div(mul(i128::from(pulse_index), i128::from(interval_ms))?, 100)?)?;
    applied
        .0
        .checked_add(offset)
        .map(Tick)
        .ok_or(StatError::Overflow)
}

/// Signed periodic resource amount; PER multiplies the current Q resource with no /100.
/// DIFF ignores current resource. `ticks/TickHp.java:61-79`, `ticks/TickMp.java:53-60`.
pub fn periodic_amount(
    power: ExactRatio,
    multiplier: ExactRatio,
    current_q: i64,
    per: bool,
) -> Result<ExactRatio, StatError> {
    let power = power.binary64()?;
    let scaled_power = if per {
        ExactRatio::new(i128::from(current_q), i128::from(Q))?
            .binary64()?
            .checked_mul(power)?
            .binary64()?
    } else {
        power
    };
    scaled_power.checked_mul(multiplier)?.binary64()
}

/// `TickHp`'s nonlethal clamp precedes CP absorption, even for a capped zero call.
/// Returns whole damage; unlike restoration, damage never stores fractional credit.
pub fn nonlethal_periodic_damage(magnitude: ExactRatio, current_hp: u32) -> Result<u32, StatError> {
    narrow(
        magnitude
            .clamp(
                ExactRatio::whole(0),
                ExactRatio::whole(i64::from(current_hp.saturating_sub(1))),
            )?
            .trunc()?,
    )
}

/// Player karma gain from PRE-increment PK count. Summons are outside Phase 3.
/// `Formulas.java:1830-1848`.
pub fn karma_gain(pk_count: u32) -> u32 {
    if pk_count < 99 {
        pk_count.saturating_mul(360).saturating_add(720)
    } else if pk_count < 180 {
        pk_count.saturating_mul(90).saturating_add(27_180)
    } else {
        43_200
    }
}

/// XP-based karma decay; exact level multiplier is not rounded to Q. `Formulas.java:1814-1821`.
/// Compound-assignment truncation occurs after the configured XP/rate division.
pub fn karma_lost(
    xp: u64,
    level_multiplier: ExactRatio,
    configured_rate: ExactRatio,
) -> Result<u64, StatError> {
    let adjusted = ExactRatio::new(i128::from(xp), 1)?
        .binary64()?
        .checked_div(configured_rate.binary64()?)?
        .binary64()?
        .trunc()?;
    narrow(
        ExactRatio::new(adjusted, 1)?
            .binary64()?
            .checked_div(level_multiplier.binary64()?)?
            .binary64()?
            .checked_div(ExactRatio::whole(30))?
            .binary64()?
            .clamp(ExactRatio::whole(0), ExactRatio::whole(i64::from(i32::MAX)))?
            .trunc()?,
    )
}

/// Pinned resurrection >20 quirk: add 20 to the multiplied result, then clamp [base,90].
/// Base 0/100 bypass this path. `Formulas.java:1372-1386`.
pub fn resurrect_percent(base: ExactRatio, wit_bonus: ExactRatio) -> Result<ExactRatio, StatError> {
    if base == ExactRatio::whole(0) || base == ExactRatio::whole(100) {
        return Ok(base);
    }
    let mut restore = base.checked_mul(wit_bonus)?;
    if restore
        .checked_sub(base)?
        .checked_cmp(ExactRatio::whole(20))?
        .is_gt()
    {
        restore = restore.checked_add(ExactRatio::whole(20))?;
    }
    // Source performs max(base) followed by min(90), even when base itself exceeds90.
    if restore.checked_cmp(base)?.is_lt() {
        restore = base;
    }
    if restore.checked_cmp(ExactRatio::whole(90))?.is_gt() {
        restore = ExactRatio::whole(90);
    }
    Ok(restore)
}

/// Refund uses Java Math.round, bounded by actual loss. `L2PcInstance.java:4700`.
pub fn resurrect_xp(loss: u64, percent: ExactRatio) -> Result<u64, StatError> {
    let refund: u64 = narrow(
        ExactRatio::new(i128::from(loss), 1)?
            .checked_mul(percent)?
            .checked_div(ExactRatio::whole(100))?
            .round()?,
    )?;
    Ok(refund.min(loss))
}

/// `PhysicalAttack.java:50,87-92` / `Formulas.java:979,1388-1405`: evasion precedes
/// critical and an evaded attack has no critical draw. Magic/debuff bypass evasion;
/// nonpositive critical chance bypasses critical. The continuous source critical roll
/// is represented in Q-percent units per the Phase 3 deterministic RNG contract.
/// `None` means evaded; `Some` carries the landed critical flag.
pub fn physical_skill_checks(
    magic: bool,
    debuff: bool,
    evasion: ExactRatio,
    critical_chance: i32,
    strength_bonus: ExactRatio,
    mut draw: impl FnMut(u32) -> Result<u32, StatError>,
) -> Result<Option<bool>, StatError> {
    let mut roll = |bound| {
        let value = draw(bound)?;
        if value >= bound {
            return Err(StatError::Overflow);
        }
        Ok(value)
    };
    if !magic
        && !debuff
        && evasion
            .checked_cmp(ExactRatio::whole(i64::from(roll(100)?)))?
            .is_gt()
    {
        return Ok(None);
    }
    let critical = critical_chance > 0
        && strength_bonus
            .checked_mul(ExactRatio::whole(i64::from(critical_chance)))?
            .checked_cmp(ExactRatio::new(i128::from(roll(100_000_000)?), i128::from(Q))?)?
            .is_gt();
    Ok(Some(critical))
}

/// Source physical skill base with caller-resolved proximity and shot state.
/// Other trait/attribute/spread/penalty modifiers are applied before `max(1, damage)`
/// by the resolver. `Formulas.java:746-809`; critical doubling is in `PhysicalAttack`.
pub fn physical_skill_base(
    power: ExactRatio,
    attack: ExactRatio,
    defence: ExactRatio,
    proximity: ExactRatio,
    soulshot: bool,
) -> Result<ExactRatio, StatError> {
    power
        .checked_add(attack.checked_mul(ExactRatio::whole(if soulshot { 2 } else { 1 }))?)?
        .checked_mul(ExactRatio::whole(77))?
        .checked_div(defence)?
        .checked_mul(proximity)
}

/// `FuncMAtkMod.java:42-47`: this function applies to the incoming initial value.
pub fn magic_attack_mod(
    initial: ExactRatio,
    intelligence: ExactRatio,
    level: ExactRatio,
) -> Result<ExactRatio, StatError> {
    initial
        .checked_mul(intelligence)?
        .checked_mul(intelligence)?
        .checked_mul(level)?
        .checked_mul(level)
}

/// `FuncMDefMod.java:58-61`: MEN and level multiply the current function value.
pub fn magic_defence_mod(
    value: ExactRatio,
    mentality: ExactRatio,
    level: ExactRatio,
) -> Result<ExactRatio, StatError> {
    value.checked_mul(mentality)?.checked_mul(level)
}

/// `FuncMAtkSpeed.java:42-45`: return the unrounded function value; the final getter
/// truncates after the configured speed cap (pinned default 1999).
pub fn magic_speed_mod(initial: ExactRatio, wit: ExactRatio) -> Result<ExactRatio, StatError> {
    initial.checked_mul(wit)
}

/// `Formulas.java:263-460`: initial stat-scaled regeneration, before generic Funcs.
/// For an NPC callers supply level/stat multipliers of one; movement is applied later.
pub fn regen_initial(
    base: ExactRatio,
    level: ExactRatio,
    stat_bonus: ExactRatio,
) -> Result<ExactRatio, StatError> {
    let initial = base.checked_mul(level)?.checked_mul(stat_bonus)?;
    Ok(if initial.checked_cmp(ExactRatio::whole(1))?.is_lt() {
        ExactRatio::whole(1)
    } else {
        initial
    })
}

/// `Formulas.java:1144-1148`: duration uses skill magic level only when positive.
pub fn duration_level_mod(
    magic_level: i32,
    attacker_level: u32,
    target_level: u32,
    bonus_rate: i32,
) -> Result<ExactRatio, StatError> {
    let level = if magic_level > 0 {
        i64::from(magic_level)
    } else {
        i64::from(attacker_level)
    };
    ExactRatio::new(add(100, i128::from(bonus_rate))?, 100)?.checked_mul(ExactRatio::new(
        add(100, sub(i128::from(level), i128::from(target_level))?)?,
        100,
    )?)
}

/// `Formulas.java:1760-1784`: debuffs ceil AFTER the half-duration/full-duration clamp.
/// Passive/permanent lifetimes are classified before calling this finite-duration helper.
pub fn effect_duration_seconds(
    time: u32,
    is_debuff: bool,
    level_mod: ExactRatio,
    stat_bonus: ExactRatio,
    trait_mod: ExactRatio,
    element_mod: ExactRatio,
) -> Result<u32, StatError> {
    if !is_debuff {
        return Ok(time);
    }
    let duration = ExactRatio::whole(i64::from(time));
    narrow(
        duration
            .checked_mul(trait_mod)?
            .checked_mul(level_mod)?
            .checked_mul(element_mod)?
            .checked_div(stat_bonus)?
            .clamp(duration.checked_div(ExactRatio::whole(2))?, duration)?
            .ceil()?,
    )
}

/// `L2Character.java:2098-2124`, pinned RespawnRestoreHP=0.65, MP/CP=0.00.
/// Return only the new HP: MP/CP are unchanged and must never be reset by this hook.
pub fn skill_revive_hp(current_hp: u32, max_hp: u32) -> Result<u32, StatError> {
    let restored = narrow(
        ExactRatio::whole(i64::from(max_hp))
            .checked_mul(ExactRatio::new(13, 20)?)?
            .trunc()?,
    )?;
    Ok(current_hp.max(restored))
}

/// Source stun break branch. A draw is required iff eligible; status/cause gates are
/// evaluated by the hit ledger before calling this helper. `PcStatus.java:83-96` and
/// `CharStatus.java:142-146`. DOT and HP-consumption never consume this roll.
pub fn stun_removed(eligible: bool, roll: Option<u32>) -> Result<bool, StatError> {
    match (eligible, roll) {
        (false, None) => Ok(false),
        (true, Some(value)) if value < 10 => Ok(value == 0),
        (_, Some(value)) => Err(StatError::DrawOutOfRange(i64::from(value))),
        (true, None) => Err(StatError::DrawOutOfRange(-1)),
    }
}

/// Source magical success rate before the integer roll. `Formulas.java:1286-1314`.
/// The explicit Java float cast is a binary32 rounding boundary; the final rate is not
/// clamped to [0,100]. Caller resolves source level fallback and NPC resistance penalties.
pub fn magic_success_rate(
    level_difference: i32,
    resistance: ExactRatio,
    target_modifier: ExactRatio,
) -> Result<i32, StatError> {
    if !(-84..=84).contains(&level_difference) {
        return Err(StatError::Overflow);
    }
    let mut factor = ExactRatio::whole(1);
    let base = if level_difference < 0 {
        ExactRatio::new(10, 13)?
    } else {
        ExactRatio::new(13, 10)?
    };
    for _ in 0..level_difference.unsigned_abs() {
        // Bound the significand after every product instead of growing 13^84.
        // The full-domain independent Fraction oracle verifies the final binary32
        // boundary, including NPC chance multipliers and Java round saturation.
        factor = factor.checked_mul(base)?.binary64()?;
    }
    narrow(sub(
        100,
        factor
            .checked_mul(resistance)?
            .checked_mul(target_modifier)?
            .binary32()?
            .round()?
            .clamp(i128::from(i32::MIN), i128::from(i32::MAX)),
    )?)
}

/// `Formulas.java:1440-1560`. `attack` already includes matching skill attribute power;
/// `matching_element` encodes the NONE/mismatched weapon early exit, before any arithmetic.
pub fn attribute_bonus(
    attack: i32,
    defence: i32,
    matching_element: bool,
    is_pvp: bool,
) -> Result<ExactRatio, StatError> {
    if !matching_element || attack <= defence {
        return Ok(ExactRatio::whole(1));
    }
    let (a, b) = if !(-99..450).contains(&attack) {
        if defence >= 450 {
            (6909, 7800)
        } else if defence >= 350 {
            (8870, 10070)
        } else {
            (12900, 14730)
        }
    } else if attack >= 300 {
        if defence >= 300 {
            (8870, 10070)
        } else if defence >= 150 {
            (12900, 14730)
        } else {
            (25000, 28940)
        }
    } else if attack >= 150 && defence >= 150 {
        (12900, 14730)
    } else if defence >= 0 {
        (25000, 28940)
    } else {
        (40000, 55000)
    };
    let attack_plus = add(i128::from(attack), 100)?;
    let defence_plus = add(i128::from(defence), 100)?;
    let delta = ExactRatio::new(mul(attack_plus, attack_plus)?, 144)?
        .checked_mul(ExactRatio::new(a, 100_000)?)?
        .checked_sub(
            ExactRatio::new(mul(defence_plus, defence_plus)?, 169)?
                .checked_mul(ExactRatio::new(b, 100_000)?)?,
        )?;
    let difference = sub(i128::from(attack), i128::from(defence))?;
    // The <= defence early return makes the negative-difference clamp branches unreachable.
    let high = if difference >= 300 {
        100
    } else if difference >= 150 {
        70
    } else {
        40
    };
    let result = delta
        .clamp(ExactRatio::whole(-50), ExactRatio::whole(high))?
        .checked_div(ExactRatio::whole(100))?
        .checked_add(ExactRatio::whole(1))?;
    Ok(if is_pvp && result.checked_cmp(ExactRatio::whole(1))?.is_lt() {
        ExactRatio::whole(1)
    } else {
        result
    })
}

/// An exact `offset + coefficient * sqrt(radicand)` local expression. Comparing its
/// square against a rational threshold avoids an early Q square-root rounding. This is
/// not serialized state and never approximates an irrational value with a guessed epsilon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RootExpression {
    offset: ExactRatio,
    coefficient: ExactRatio,
    radicand: ExactRatio,
}

impl RootExpression {
    /// Require a real square root, retaining all immutable rational inputs exactly.
    pub fn new(
        offset: ExactRatio,
        coefficient: ExactRatio,
        radicand: ExactRatio,
    ) -> Result<Self, StatError> {
        if radicand.parts().0 < 0 {
            return Err(StatError::Overflow);
        }
        Ok(Self {
            offset,
            coefficient,
            radicand,
        })
    }
    /// `(offset, coefficient, radicand)` for evidence certificates.
    #[must_use]
    pub fn parts(self) -> (ExactRatio, ExactRatio, ExactRatio) {
        (self.offset, self.coefficient, self.radicand)
    }
    /// A rational branch such as full magic resist or a clamped rate.
    pub fn constant(value: ExactRatio) -> Self {
        Self {
            offset: value,
            coefficient: ExactRatio::whole(0),
            radicand: ExactRatio::whole(0),
        }
    }
    /// Compare before narrowing, preserving integer-roll boundary behavior.
    pub fn checked_cmp(self, rhs: ExactRatio) -> Result<std::cmp::Ordering, StatError> {
        let mut threshold = rhs.checked_sub(self.offset)?;
        let sign = self.coefficient.parts().0.cmp(&0);
        if sign.is_eq() || self.radicand.parts().0 == 0 {
            return self.offset.checked_cmp(rhs);
        }
        if sign.is_lt() {
            threshold = ExactRatio::whole(0).checked_sub(threshold)?;
        }
        let result = if threshold.parts().0 < 0 {
            std::cmp::Ordering::Greater
        } else {
            self.coefficient
                .checked_mul(self.coefficient)?
                .checked_mul(self.radicand)?
                .checked_cmp(threshold.checked_mul(threshold)?)?
        };
        Ok(if sign.is_lt() {
            result.reverse()
        } else {
            result
        })
    }
    /// Apply a source multiplier to the whole expression.
    pub fn checked_mul(self, factor: ExactRatio) -> Result<Self, StatError> {
        Self::new(
            self.offset.checked_mul(factor)?,
            self.coefficient.checked_mul(factor)?,
            self.radicand,
        )
    }
    /// Source clamp with no preliminary square-root conversion.
    pub fn clamp(self, low: ExactRatio, high: ExactRatio) -> Result<Self, StatError> {
        if low.checked_cmp(high)?.is_gt() {
            return Err(StatError::Overflow);
        }
        Ok(if self.checked_cmp(low)?.is_lt() {
            Self::constant(low)
        } else if self.checked_cmp(high)?.is_gt() {
            Self::constant(high)
        } else {
            self
        })
    }
    /// Greatest integer <= scale * expression, found inside proved root bounds.
    pub fn floor_scaled(self, scale: u32) -> Result<i64, StatError> {
        if scale == 0 {
            return Err(StatError::NonPositiveDivisor);
        }
        let whole: u128 = narrow(self.radicand.floor()?)?;
        let root = i128::try_from(super::scaled::isqrt(whole)).map_err(|_| StatError::Overflow)?;
        let lower = self
            .offset
            .checked_add(self.coefficient.checked_mul(ExactRatio::new(root, 1)?)?)?;
        let upper = self.offset.checked_add(
            self.coefficient
                .checked_mul(ExactRatio::new(add(root, 1)?, 1)?)?,
        )?;
        let (lower, upper) = if self.coefficient.parts().0 < 0 {
            (upper, lower)
        } else {
            (lower, upper)
        };
        let multiplier = ExactRatio::whole(i64::from(scale));
        let mut low: i64 = narrow(lower.checked_mul(multiplier)?.floor()?)?;
        let mut high: i64 = narrow(upper.checked_mul(multiplier)?.ceil()?)?;
        while low < high {
            let middle: i64 = narrow(add(
                i128::from(low),
                ceil_div(sub(i128::from(high), i128::from(low))?, 2)?,
            )?)?;
            if self
                .checked_cmp(ExactRatio::new(i128::from(middle), i128::from(scale))?)?
                .is_ge()
            {
                low = middle;
            } else {
                high = middle.checked_sub(1).ok_or(StatError::Overflow)?;
            }
        }
        Ok(low)
    }
    /// Final Java Math.round boundary (ties toward positive infinity).
    pub fn round(self) -> Result<i64, StatError> {
        Self::new(
            self.offset.checked_add(ExactRatio::new(1, 2)?)?,
            self.coefficient,
            self.radicand,
        )?
        .floor_scaled(1)
    }
}

/// Already-resolved success branch. RNG consumption belongs to the ordered hit ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagicResolution {
    /// Full damage, with optional source critical multiplier.
    Success,
    /// Failed first roll and successful eligible second roll: half damage, no critical.
    Half,
    /// Full resistance: constant one before post-resolution modifiers.
    Resist,
}

/// `Formulas.java:815-898`; no shots/shields in Phase 3. Apply spread/PvP/attribute/penalty
/// multipliers to this expression before whole truncation in the handler.
pub fn magical_skill_base(
    power: ExactRatio,
    attack: ExactRatio,
    defence: ExactRatio,
    resolution: MagicResolution,
    critical: bool,
    is_pvp: bool,
) -> Result<RootExpression, StatError> {
    if resolution == MagicResolution::Resist {
        return Ok(RootExpression::constant(ExactRatio::whole(1)));
    }
    let factor = if resolution == MagicResolution::Half {
        ExactRatio::new(1, 2)?
    } else if critical {
        if is_pvp {
            ExactRatio::new(5, 2)?
        } else {
            ExactRatio::whole(3)
        }
    } else {
        ExactRatio::whole(1)
    };
    RootExpression::new(
        ExactRatio::whole(0),
        power
            .checked_mul(ExactRatio::whole(91))?
            .checked_div(defence)?
            .checked_mul(factor)?,
        attack,
    )
}

/// `Formulas.java:992-1025`; caller handles casting/channel/raid/invulnerability gates and
/// the source `ATTACK_CANCEL` modifiers. This helper retains the fractional comparison.
pub fn attack_break_base(damage: u32, men_bonus: ExactRatio) -> Result<RootExpression, StatError> {
    RootExpression::new(
        ExactRatio::whole(115).checked_sub(men_bonus.checked_mul(ExactRatio::whole(100))?)?,
        ExactRatio::whole(1),
        ExactRatio::whole(i64::from(damage)).checked_mul(ExactRatio::whole(13))?,
    )
}

/// `FuncAtkEvasion.java:42-61` prior to its intermediate `(int)` cast at priority one.
/// Call `floor_scaled(1)` for the nonnegative built-in, then install later MUL/ADD Funcs.
pub fn evasion_builtin(
    initial: ExactRatio,
    dex: u32,
    level: u32,
    player: bool,
) -> Result<RootExpression, StatError> {
    let mut offset = initial.checked_add(ExactRatio::whole(i64::from(level)))?;
    if level >= 70 {
        let extra = ExactRatio::new(sub(i128::from(level), 69)?, 1)?;
        offset = offset.checked_add(if player {
            if level >= 78 {
                extra.checked_mul(ExactRatio::new(6, 5)?)?
            } else {
                extra
            }
        } else {
            extra.checked_add(ExactRatio::whole(2))?
        })?;
    }
    RootExpression::new(offset, ExactRatio::whole(6), ExactRatio::whole(i64::from(dex)))
}

/// `Heal.java:84-102`: dynamic power plus root, before `HEAL_EFFECT`, critical and cap. Static
/// skills bypass root and modifiers; the caller preserves that source branch explicitly.
pub fn heal_base(
    power: ExactRatio,
    attack: ExactRatio,
    weapon_multiplier: u32,
    is_static: bool,
) -> Result<RootExpression, StatError> {
    if is_static {
        return Ok(RootExpression::constant(power));
    }
    RootExpression::new(
        power,
        ExactRatio::whole(1),
        attack.checked_mul(ExactRatio::whole(i64::from(weapon_multiplier)))?,
    )
}

/// Resolved source inputs for `Formulas.calcEffectSuccess:1158-1232`. Basic property NONE
/// is an explicit unconditional branch, not a synthetic zero target base stat.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[allow(missing_docs)]
pub struct EffectSuccessInputs {
    pub magic_level: i32,
    pub target_level: u32,
    pub activate_rate: i32,
    pub level_bonus_rate: i32,
    pub target_base_stat: u32,
    pub basic_property_none: bool,
    pub is_magic: bool,
    pub magic_attack: ExactRatio,
    pub magic_defence: ExactRatio,
    pub trait_modifier: ExactRatio,
    pub element_modifier: ExactRatio,
    pub buff_debuff_modifier: ExactRatio,
    pub min_chance: u32,
    pub max_chance: u32,
}

/// Effect landing rate, before the ledger's integer roll. Blocked target kinds/debuff
/// immunity are eligibility gates before the unconditional branch and before RNG.
pub fn effect_success_rate(p: EffectSuccessInputs) -> Result<RootExpression, StatError> {
    if p.activate_rate == -1 || p.basic_property_none {
        return Ok(RootExpression::constant(ExactRatio::whole(100)));
    }
    if p.trait_modifier.parts().0 <= 0 {
        return Ok(RootExpression::constant(ExactRatio::whole(0)));
    }
    let level = if p.magic_level <= -1 {
        add(i128::from(p.target_level), 3)?
    } else {
        i128::from(p.magic_level)
    };
    let base = sub(
        add(
            mul(add(sub(level, i128::from(p.target_level))?, 3)?, i128::from(p.level_bonus_rate))?,
            add(i128::from(p.activate_rate), 30)?,
        )?,
        i128::from(p.target_base_stat),
    )?;
    let coefficient = ExactRatio::new(base, 1)?
        .checked_mul(p.trait_modifier)?
        .checked_mul(p.element_modifier)?
        .checked_mul(p.buff_debuff_modifier)?;
    let rate = if p.is_magic {
        RootExpression::new(
            ExactRatio::whole(0),
            coefficient
                .checked_div(p.magic_defence)?
                .checked_mul(ExactRatio::whole(11))?,
            p.magic_attack,
        )?
    } else {
        RootExpression::constant(coefficient)
    };
    rate.clamp(
        ExactRatio::whole(i64::from(p.min_chance)),
        ExactRatio::whole(i64::from(p.max_chance)),
    )
}

/// Result of a Q restoration pulse. `granted` counts newly materialized whole units only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Restoration {
    pub current: u32,
    pub remainder_q: u32,
    pub granted: u32,
}

/// Whole resource plus canonical Q remainder. Preserve an already-above-cap resource;
/// clear fractions at a full clamp. Callers exclude dead entities before accruing credit.
pub fn restore_resource(
    current: u32,
    maximum: u32,
    remainder_q: u32,
    amount_q: i64,
) -> Result<Restoration, StatError> {
    if i64::from(remainder_q) >= Q || amount_q < 0 {
        return Err(StatError::Overflow);
    }
    if current >= maximum {
        return Ok(Restoration {
            current,
            remainder_q: 0,
            granted: 0,
        });
    }
    let accumulated = add(i128::from(remainder_q), i128::from(amount_q))?;
    let potential = accumulated
        .checked_div(i128::from(Q))
        .ok_or(StatError::Overflow)?;
    let gap = sub(i128::from(maximum), i128::from(current))?;
    let granted = narrow::<u32>(potential.min(gap))?;
    let remainder_q = if potential >= gap {
        0
    } else {
        narrow(
            accumulated
                .checked_rem(i128::from(Q))
                .ok_or(StatError::Overflow)?,
        )?
    };
    Ok(Restoration {
        current: current.checked_add(granted).ok_or(StatError::Overflow)?,
        remainder_q,
        granted,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn ratio(value: &Value) -> ExactRatio {
        if let Some(n) = value.as_i64() {
            return ExactRatio::whole(n);
        }
        let raw = value.as_str().unwrap();
        if let Some((n, d)) = raw.split_once('/') {
            ExactRatio::new(n.parse().unwrap(), d.parse().unwrap()).unwrap()
        } else if let Some((n, d)) = raw.split_once('.') {
            let scale = 10_i128.pow(u32::try_from(d.len()).unwrap());
            let numerator = n.parse::<i128>().unwrap() * scale
                + if n.starts_with('-') {
                    -d.parse::<i128>().unwrap()
                } else {
                    d.parse::<i128>().unwrap()
                };
            ExactRatio::new(numerator, scale).unwrap()
        } else {
            ExactRatio::whole(raw.parse().unwrap())
        }
    }
    fn uint(value: &Value) -> u32 {
        u32::try_from(value.as_u64().unwrap()).unwrap()
    }
    fn boolean(value: &Value) -> bool {
        value.as_bool().unwrap()
    }

    #[test]
    fn physical_caller_injected_roll_trace_and_strict_q_boundaries() {
        for (magic, debuff, evasion, chance, input, expected, bounds) in [
            (false, false, 0, 0, vec![0], Some(false), vec![100]),
            (false, false, 0, -1, vec![0], Some(false), vec![100]),
            (false, false, 1, 100, vec![0], None, vec![100]),
            (false, false, 1, 50, vec![1, 49_999_999], Some(true), vec![100, 100_000_000]),
            (false, false, 1, 50, vec![1, 50_000_000], Some(false), vec![100, 100_000_000]),
            (true, false, 100, 50, vec![0], Some(true), vec![100_000_000]),
            (false, true, 100, 0, vec![], Some(false), vec![]),
        ] {
            let mut rolls = input.into_iter();
            let mut trace = Vec::new();
            let actual = physical_skill_checks(
                magic,
                debuff,
                ExactRatio::whole(evasion),
                chance,
                ExactRatio::whole(1),
                |bound| {
                    trace.push(bound);
                    Ok(rolls.next().unwrap())
                },
            )
            .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(trace, bounds);
            assert_eq!(rolls.next(), None);
        }
        assert!(physical_skill_checks(
            false,
            false,
            ExactRatio::whole(0),
            0,
            ExactRatio::whole(1),
            |_| Ok(100)
        )
        .is_err());
    }

    #[test]
    fn oracle_manifest_has_exactly_the_families_exercised_below() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        let formulas = fixture["formulas"].as_object().unwrap();
        let expected: std::collections::BTreeSet<_> = [
            "calcSkillPhysDam",
            "calcMagicDam",
            "calcMagicSuccess",
            "calcEffectSuccess",
            "effect_duration",
            "calcAtkBreak",
            "calcAttributeBonus",
            "calcHpRegen",
            "calcMpRegen",
            "calcCpRegen",
            "calculateKarmaGain",
            "calculateKarmaLost",
            "calculateSkillResurrectRestorePercent",
            "calcCastTime",
            "doRevive",
            "FuncMAtkMod",
            "FuncMDefMod",
            "FuncMAtkSpeed",
            "TickHp",
            "cast_staging",
            "Heal",
            "FuncAtkEvasion",
            "stun_removal_condition",
            "getAffectLimit",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            formulas
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        assert_eq!(
            formulas
                .values()
                .map(|rows| rows.as_array().unwrap().len())
                .sum::<usize>(),
            1203
        );
    }

    #[test]
    fn independent_oracle_cast_limit_karma_and_resurrection_vectors() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        let mut checked = 0;
        for family in [
            "calcCastTime",
            "getAffectLimit",
            "calculateKarmaGain",
            "calculateKarmaLost",
            "calculateSkillResurrectRestorePercent",
        ] {
            for vector in fixture["formulas"][family].as_array().unwrap() {
                let p = &vector["inputs"];
                let expected = &vector["rounded_result"];
                match family {
                    "calcCastTime" => {
                        let animation = cast_animation(
                            uint(&p["hit_time"]),
                            uint(&p["speed"]),
                            boolean(&p["is_static"]),
                            boolean(&p["is_magic"]),
                            boolean(&p["charged_shots"]),
                        )
                        .unwrap();
                        assert_eq!(
                            animation.trunc().unwrap(),
                            i128::from(expected.as_i64().unwrap()),
                            "{vector}"
                        );
                        assert_eq!(
                            ceil_div(animation.trunc().unwrap(), 100).unwrap(),
                            i128::from(vector["ticks_result"].as_i64().unwrap()),
                            "{vector}"
                        );
                    },
                    "getAffectLimit" => {
                        let roll = p["roll_draw"].as_u64().map(|v| u32::try_from(v).unwrap());
                        let limit =
                            affect_limit(uint(&p["offset"]), uint(&p["width"]), roll).unwrap();
                        assert_eq!(limit, uint(&expected["limit_return"]), "{vector}");
                        assert_eq!(limit == 0, boolean(&expected["is_unlimited"]), "{vector}");
                    },
                    "calculateKarmaGain" => {
                        assert_eq!(karma_gain(uint(&p["pk_count"])), uint(expected), "{vector}");
                    },
                    "calculateKarmaLost" => assert_eq!(
                        karma_lost(
                            p["exp"].as_u64().unwrap(),
                            ratio(&p["karma_multiplier"]),
                            ExactRatio::whole(1)
                        )
                        .unwrap(),
                        expected.as_u64().unwrap(),
                        "{vector}"
                    ),
                    "calculateSkillResurrectRestorePercent" => {
                        let percent = resurrect_percent(
                            ratio(&p["base_restore_percent"]),
                            ratio(&p["wit_bonus"]),
                        )
                        .unwrap();
                        assert_eq!(percent, ratio(&vector["exact_rational"]), "{vector}");
                        assert_eq!(
                            resurrect_xp(p["recoverable_loss_example"].as_u64().unwrap(), percent)
                                .unwrap(),
                            vector["xp_refund_example"].as_u64().unwrap(),
                            "{vector}"
                        );
                    },
                    _ => unreachable!(),
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 275);
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One explicit input adapter per independent oracle family.
    fn independent_oracle_modifiers_regen_duration_physical_revive_and_stun() {
        use crate::domain::zone::StatKind;
        use crate::infrastructure::rules_data::{load_rules, RulesSource};
        let rules = load_rules(&RulesSource::embedded()).unwrap().rules;
        let bonus = |kind, value| {
            ExactRatio::new(
                i128::from(rules.bonus().bonus(kind, value).unwrap().raw()),
                i128::from(Q),
            )
            .unwrap()
        };
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        let mut checked = 0;
        for family in [
            "calcSkillPhysDam",
            "FuncMAtkMod",
            "FuncMDefMod",
            "FuncMAtkSpeed",
            "calcHpRegen",
            "calcMpRegen",
            "calcCpRegen",
            "effect_duration",
            "doRevive",
            "stun_removal_condition",
        ] {
            for vector in fixture["formulas"][family].as_array().unwrap() {
                let p = &vector["inputs"];
                let level = || ExactRatio::new(i128::from(uint(&p["level"])) + 89, 100).unwrap();
                let calculated = match family {
                    "calcSkillPhysDam" => {
                        let proximity = match p["proximity"].as_str().unwrap() {
                            "front" => ExactRatio::whole(1),
                            "side" => ExactRatio::new(11, 10).unwrap(),
                            "behind" => ExactRatio::new(6, 5).unwrap(),
                            other => unreachable!("unexpected proximity {other}"),
                        };
                        let base = physical_skill_base(
                            ratio(&p["power"]),
                            ratio(&p["p_atk"]),
                            ratio(&p["defence"]),
                            proximity,
                            boolean(&p["soulshot"]),
                        )
                        .unwrap();
                        let base = if base.checked_cmp(ExactRatio::whole(1)).unwrap().is_lt() {
                            ExactRatio::whole(1)
                        } else {
                            base
                        };
                        base.checked_mul(ExactRatio::whole(if boolean(&p["crit"]) { 2 } else { 1 }))
                            .unwrap()
                    },
                    "FuncMAtkMod" => magic_attack_mod(
                        ratio(&p["init_val"]),
                        bonus(StatKind::Int, uint(&p["int_stat"])),
                        level(),
                    )
                    .unwrap(),
                    "FuncMDefMod" => magic_defence_mod(
                        ratio(&p["value"]),
                        bonus(StatKind::Men, uint(&p["men_stat"])),
                        level(),
                    )
                    .unwrap(),
                    "FuncMAtkSpeed" => magic_speed_mod(
                        ratio(&p["init_val"]),
                        bonus(StatKind::Wit, uint(&p["wit_stat"])),
                    )
                    .unwrap()
                    .clamp(ExactRatio::whole(0), ratio(&p["max_m_atk_speed"]))
                    .unwrap(),
                    "calcHpRegen" | "calcMpRegen" | "calcCpRegen" => {
                        let player = boolean(&p["is_player"]);
                        let stat = if family == "calcMpRegen" {
                            StatKind::Men
                        } else {
                            StatKind::Con
                        };
                        let initial = regen_initial(
                            ratio(&p["base_regen"]),
                            if player {
                                level()
                            } else {
                                ExactRatio::whole(1)
                            },
                            if player {
                                bonus(stat, uint(&p["stat_value"]))
                            } else {
                                ExactRatio::whole(1)
                            },
                        )
                        .unwrap();
                        let movement = if player {
                            match p["posture"].as_str().unwrap() {
                                "standing" => ExactRatio::new(11, 10).unwrap(),
                                "moving" => ExactRatio::new(7, 10).unwrap(),
                                "sitting" => ExactRatio::new(3, 2).unwrap(),
                                other => unreachable!("unknown posture {other}"),
                            }
                        } else {
                            ExactRatio::whole(1)
                        };
                        initial.checked_mul(movement).unwrap()
                    },
                    "effect_duration" => {
                        let level = duration_level_mod(
                            i32::try_from(p["magic_level"].as_i64().unwrap()).unwrap(),
                            uint(&p["attacker_level"]),
                            uint(&p["target_level"]),
                            i32::try_from(p["lvl_bonus_rate"].as_i64().unwrap()).unwrap(),
                        )
                        .unwrap();
                        let result = effect_duration_seconds(
                            uint(&p["abnormal_time"]),
                            boolean(&p["is_debuff"]),
                            level,
                            ratio(&p["stat_bonus"]),
                            ratio(&p["trait_mod"]),
                            ratio(&p["element_mod"]),
                        )
                        .unwrap();
                        assert_eq!(result, uint(&vector["rounded_result"]), "{vector}");
                        assert_eq!(result * 10, uint(&vector["ticks_result"]), "{vector}");
                        checked += 1;
                        continue;
                    },
                    "doRevive" => {
                        assert_eq!(
                            skill_revive_hp(uint(&p["current_hp"]), uint(&p["max_hp"])).unwrap(),
                            uint(&vector["rounded_result"]["hp"]),
                            "{vector}"
                        );
                        assert_eq!(p["current_mp"], vector["rounded_result"]["mp"], "{vector}");
                        assert_eq!(p["current_cp"], vector["rounded_result"]["cp"], "{vector}");
                        checked += 1;
                        continue;
                    },
                    "stun_removal_condition" => {
                        let eligible = boolean(&p["target_is_stunned"])
                            && !["DOT", "HP_CONSUMPTION"]
                                .contains(&p["damage_cause"].as_str().unwrap());
                        let roll = if eligible {
                            Some(uint(&p["roll_draw"]))
                        } else {
                            None
                        };
                        let removed = stun_removed(eligible, roll).unwrap();
                        assert_eq!(
                            eligible,
                            boolean(&vector["rounded_result"]["roll_consumed"]),
                            "{vector}"
                        );
                        assert_eq!(
                            removed,
                            boolean(&vector["rounded_result"]["stun_removed"]),
                            "{vector}"
                        );
                        assert_eq!(
                            boolean(&p["target_is_stunned"]) && !removed,
                            boolean(&vector["rounded_result"]["final_is_stunned"]),
                            "{vector}"
                        );
                        checked += 1;
                        continue;
                    },
                    _ => unreachable!(),
                };
                if let Some(pulses) = vector.get("pulse_trace").and_then(Value::as_array) {
                    let (mut current, mut remainder) = (0, 0);
                    for pulse in pulses {
                        let result = restore_resource(
                            current,
                            u32::MAX,
                            remainder,
                            calculated.to_q().unwrap(),
                        )
                        .unwrap();
                        assert_eq!(result.granted, uint(&pulse["whole_granted"]), "{vector}");
                        assert_eq!(result.current, uint(&pulse["current"]), "{vector}");
                        assert_eq!(result.remainder_q, uint(&pulse["remainder_q"]), "{vector}");
                        current = result.current;
                        remainder = result.remainder_q;
                    }
                }
                assert_eq!(
                    calculated.to_q().unwrap(),
                    vector["q_result"].as_i64().unwrap(),
                    "{family}: {vector}"
                );
                assert_eq!(
                    calculated.trunc().unwrap(),
                    i128::from(vector["rounded_result"].as_i64().unwrap()),
                    "{family}: {vector}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 479);
    }

    #[test]
    fn magic_success_full_admitted_gap_oracle() {
        let vectors: Value =
            serde_json::from_str(include_str!("../../../fixtures/magic-success/vectors.json"))
                .unwrap();
        for vector in vectors.as_array().unwrap() {
            let gap = i32::try_from(vector["gap"].as_i64().unwrap()).unwrap();
            let modifier = ratio(&vector["modifier"]);
            let expected = i32::try_from(vector["rate"].as_i64().unwrap()).unwrap();
            assert_eq!(
                magic_success_rate(gap, ExactRatio::whole(1), modifier).unwrap(),
                expected,
                "{vector}"
            );
            assert_eq!(
                magic_success_rate(gap, modifier, ExactRatio::whole(1)).unwrap(),
                expected,
                "{vector}"
            );
        }
    }

    #[test]
    fn independent_oracle_magic_success_and_attribute_vectors() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        for vector in fixture["formulas"]["calcMagicSuccess"].as_array().unwrap() {
            let p = &vector["inputs"];
            assert_eq!(
                magic_success_rate(
                    i32::try_from(p["lvl_difference"].as_i64().unwrap()).unwrap(),
                    ratio(&p["res_modifier"]),
                    ratio(&p["target_modifier"])
                )
                .unwrap(),
                i32::try_from(vector["rounded_result"].as_i64().unwrap()).unwrap(),
                "{vector}"
            );
        }
        for vector in fixture["formulas"]["calcAttributeBonus"]
            .as_array()
            .unwrap()
        {
            let p = &vector["inputs"];
            let matching = p["weapon_element"].as_i64().is_some_and(|v| v >= 0)
                && p["weapon_element"] == p["skill_element"];
            let value = attribute_bonus(
                i32::try_from(p["attack_attribute"].as_i64().unwrap()).unwrap(),
                i32::try_from(p["defence_attribute"].as_i64().unwrap()).unwrap(),
                matching,
                boolean(&p["is_pvp"]),
            )
            .unwrap();
            assert_eq!(value, ratio(&vector["exact_rational"]), "{vector}");
            assert_eq!(value.to_q().unwrap(), vector["q_result"].as_i64().unwrap(), "{vector}");
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Formula input dispatch keeps expectations separate.
    fn independent_oracle_root_formula_vectors() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        for family in [
            "calcMagicDam",
            "calcAtkBreak",
            "calcEffectSuccess",
            "Heal",
            "FuncAtkEvasion",
        ] {
            for vector in fixture["formulas"][family].as_array().unwrap() {
                let p = &vector["inputs"];
                let signed = |name: &str| i32::try_from(p[name].as_i64().unwrap()).unwrap();
                let result = match family {
                    "calcMagicDam" => {
                        let resolution = if !boolean(&p["magic_failure"]) {
                            MagicResolution::Success
                        } else if boolean(&p["second_success"]) && signed("lvl_diff") <= 9 {
                            MagicResolution::Half
                        } else {
                            MagicResolution::Resist
                        };
                        magical_skill_base(
                            ratio(&p["power"]),
                            ratio(&p["m_atk"]),
                            ratio(&p["m_def"]),
                            resolution,
                            boolean(&p["mcrit"]),
                            boolean(&p["is_pvp"]),
                        )
                        .unwrap()
                        .checked_mul(ratio(&p["attribute_bonus"]))
                        .unwrap()
                    },
                    "calcAtkBreak" => {
                        if boolean(&p["is_casting"]) {
                            attack_break_base(uint(&p["damage"]), ratio(&p["men_bonus"]))
                                .unwrap()
                                .clamp(ExactRatio::whole(1), ExactRatio::whole(99))
                                .unwrap()
                        } else {
                            RootExpression::constant(ExactRatio::whole(0))
                        }
                    },
                    "calcEffectSuccess" => effect_success_rate(EffectSuccessInputs {
                        magic_level: signed("magic_level"),
                        target_level: uint(&p["target_level"]),
                        activate_rate: signed("activate_rate"),
                        level_bonus_rate: signed("lvl_bonus_rate"),
                        target_base_stat: uint(&p["target_base_stat"]),
                        basic_property_none: p["basic_property"] == "NONE",
                        is_magic: boolean(&p["is_magic"]),
                        magic_attack: ratio(&p["m_atk"]),
                        magic_defence: ratio(&p["m_def"]),
                        trait_modifier: ratio(&p["trait_mod"]),
                        element_modifier: ratio(&p["element_mod"]),
                        buff_debuff_modifier: ratio(&p["buff_debuff_mod"]),
                        min_chance: uint(&p["min_chance"]),
                        max_chance: uint(&p["max_chance"]),
                    })
                    .unwrap(),
                    "Heal" => {
                        let mut multiplier = match p["weapon_grade"].as_str().unwrap() {
                            "S84" => 4,
                            "S80" => 2,
                            _ => 1,
                        };
                        multiplier = if boolean(&p["bss"]) {
                            multiplier * 4
                        } else {
                            multiplier + 1
                        };
                        let mut result = heal_base(
                            ratio(&p["power"]),
                            ratio(&p["m_atk"]),
                            multiplier,
                            boolean(&p["is_static"]),
                        )
                        .unwrap();
                        if !boolean(&p["is_static"]) {
                            result = result.checked_mul(ratio(&p["heal_effect"])).unwrap();
                            if boolean(&p["is_magic"]) && boolean(&p["is_crit"]) {
                                result = result.checked_mul(ExactRatio::whole(3)).unwrap();
                            }
                        }
                        let gap = ratio(&p["max_recoverable_hp"])
                            .checked_sub(ratio(&p["current_hp"]))
                            .unwrap();
                        let gap = if gap.parts().0 < 0 {
                            ExactRatio::whole(0)
                        } else {
                            gap
                        };
                        let amount = result
                            .clamp(ExactRatio::whole(0), gap)
                            .unwrap()
                            .floor_scaled(1)
                            .unwrap();
                        assert_eq!(
                            amount,
                            vector["rounded_result"]["heal_amount"].as_i64().unwrap(),
                            "{vector}"
                        );
                        assert_eq!(
                            amount + p["current_hp"].as_i64().unwrap(),
                            vector["rounded_result"]["final_hp"].as_i64().unwrap(),
                            "{vector}"
                        );
                        continue;
                    },
                    "FuncAtkEvasion" => {
                        let raw = evasion_builtin(
                            ExactRatio::whole(0),
                            uint(&p["dex"]),
                            uint(&p["level"]),
                            boolean(&p["is_player"]),
                        )
                        .unwrap();
                        let mut intermediate = ExactRatio::whole(raw.floor_scaled(1).unwrap());
                        let modifier = &p["modifier"];
                        intermediate = match modifier["op"].as_str().unwrap() {
                            "none" => intermediate,
                            "mul" => intermediate.checked_mul(ratio(&modifier["val"])).unwrap(),
                            "add" => intermediate.checked_add(ratio(&modifier["val"])).unwrap(),
                            other => unreachable!("unrecognized evasion modifier {other}"),
                        };
                        assert_eq!(
                            intermediate.round().unwrap(),
                            i128::from(
                                vector["rounded_result"]["evasion_with_intermediate_cast"]
                                    .as_i64()
                                    .unwrap()
                            ),
                            "{vector}"
                        );
                        continue;
                    },
                    _ => unreachable!(),
                };
                assert_eq!(
                    result.floor_scaled(1).unwrap(),
                    vector["rounded_result"].as_i64().unwrap(),
                    "{family}: {vector}"
                );
                assert_eq!(
                    result.floor_scaled(u32::try_from(Q).unwrap()).unwrap(),
                    vector["q_result"].as_i64().unwrap(),
                    "{family}: {vector}"
                );
            }
        }
    }

    #[test]
    fn square_root_boundaries_are_exact_including_negative_coefficients() {
        let root =
            RootExpression::new(ExactRatio::whole(0), ExactRatio::whole(1), ExactRatio::whole(2))
                .unwrap();
        assert_eq!(root.floor_scaled(1_000_000).unwrap(), 1_414_213);
        assert!(root
            .checked_cmp(ExactRatio::new(1_414_213, 1_000_000).unwrap())
            .unwrap()
            .is_gt());
        let negative = root.checked_mul(ExactRatio::whole(-1)).unwrap();
        assert_eq!(negative.floor_scaled(1_000_000).unwrap(), -1_414_214);
        let integer =
            RootExpression::new(ExactRatio::whole(3), ExactRatio::whole(-2), ExactRatio::whole(4))
                .unwrap();
        assert_eq!(integer.floor_scaled(1).unwrap(), -1);
        assert_eq!(integer.checked_cmp(ExactRatio::whole(-1)).unwrap(), std::cmp::Ordering::Equal);
        assert!(RootExpression::new(
            ExactRatio::whole(0),
            ExactRatio::whole(1),
            ExactRatio::whole(-1)
        )
        .is_err());
    }

    #[test]
    fn independent_oracle_cast_staging_and_periodic_traces() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        for vector in fixture["formulas"]["cast_staging"].as_array().unwrap() {
            let p = &vector["inputs"];
            let start = Tick(p["start_tick"].as_u64().unwrap());
            let animation = if boolean(&p["is_static"]) {
                ratio(&p["hit_time_ms"])
            } else {
                cast_animation(uint(&p["hit_time_ms"]), uint(&p["cast_speed"]), false, false, false)
                    .unwrap()
            };
            let result = cast_deadlines(start, animation, boolean(&p["is_continuous"])).unwrap();
            let expected = &vector["rounded_result"];
            assert_eq!(result.launch_at.0, expected["launch_tick"].as_u64().unwrap(), "{vector}");
            assert_eq!(result.complete_at.0, expected["impact_tick"].as_u64().unwrap(), "{vector}");
            assert_eq!(
                result.interrupt_until.0,
                expected["interrupt_cutoff_tick"]
                    .as_u64()
                    .unwrap()
                    .max(start.0),
                "{vector}"
            );
        }
        for vector in fixture["formulas"]["TickHp"].as_array().unwrap() {
            let p = &vector["inputs"];
            let (interval, multiplier) =
                periodic_interval(uint(&p["ticks"]), uint(&p["effect_tick_ratio"])).unwrap();
            assert_eq!(multiplier, ratio(&vector["intermediate"]["ticks_multiplier"]), "{vector}");
            let (mut hp, mut cp, mut remainder) =
                (uint(&p["current_hp"]), uint(&p["current_cp"]), 0_u32);
            for pulse in vector["pulse_trace"].as_array().unwrap() {
                let current_q = i64::from(hp) * Q + i64::from(remainder);
                let amount =
                    periodic_amount(ratio(&p["power"]), multiplier, current_q, p["mode"] == "PER")
                        .unwrap();
                assert_eq!(amount, ratio(&pulse["raw_amount"]), "{vector}");
                let (mut damage, mut cp_removed, mut healed) = (0, 0, 0);
                if amount.parts().0 < 0 {
                    damage = nonlethal_periodic_damage(
                        ExactRatio::whole(0).checked_sub(amount).unwrap(),
                        hp,
                    )
                    .unwrap();
                    if boolean(&p["player_source"]) && boolean(&p["target_is_player"]) {
                        cp_removed = damage.min(cp);
                    }
                    cp -= cp_removed;
                    hp -= damage - cp_removed;
                } else {
                    let result =
                        restore_resource(hp, uint(&p["max_hp"]), remainder, amount.to_q().unwrap())
                            .unwrap();
                    hp = result.current;
                    remainder = result.remainder_q;
                    healed = result.granted;
                }
                assert_eq!(
                    periodic_deadline(Tick(0), pulse["pulse"].as_u64().unwrap(), interval)
                        .unwrap()
                        .0,
                    pulse["tick_offset"].as_u64().unwrap(),
                    "{vector}"
                );
                assert_eq!(
                    (damage, cp_removed, healed, hp, cp, remainder),
                    (
                        uint(&pulse["whole_damage"]),
                        uint(&pulse["cp_removed"]),
                        uint(&pulse["whole_restored"]),
                        uint(&pulse["final_hp"]),
                        uint(&pulse["final_cp"]),
                        uint(&pulse["restoration_remainder_q"])
                    ),
                    "{vector}"
                );
            }
        }
    }

    #[test]
    fn source_cast_staging_has_separate_launch_impact_and_interrupt_deadlines() {
        // v3 §6.1 independently source-checked table; two continuous short-cast oracle
        // rows currently disagree and have been requested from the data owner.
        for (ms, continuous, launch, impact, interrupt) in [
            (0, false, 0, 1, 0),
            (100, false, 0, 4, 0),
            (500, false, 1, 5, 3),
            (500, true, 0, 4, 3),
            (550, false, 2, 6, 3),
            (550, true, 0, 4, 3),
            (1200, false, 8, 12, 10),
            (1200, true, 5, 9, 10),
        ] {
            assert_eq!(
                cast_deadlines(Tick(100), ExactRatio::whole(ms), continuous).unwrap(),
                CastDeadlines {
                    animation_ms: u64::try_from(ms).unwrap(),
                    launch_at: Tick(100 + launch),
                    complete_at: Tick(100 + impact),
                    interrupt_until: Tick(100 + interrupt)
                }
            );
        }
        let fractional = cast_deadlines(Tick(10), ExactRatio::new(1, 2).unwrap(), true).unwrap();
        assert_eq!(
            (fractional.launch_at, fractional.complete_at, fractional.interrupt_until),
            (Tick(10), Tick(11), Tick(10))
        );
        assert!(cast_deadlines(Tick(u64::MAX), ExactRatio::whole(500), false).is_err());
    }

    #[test]
    fn binary32_rounds_ties_to_even_and_keeps_exact_widened_multiplier() {
        assert_eq!(
            ExactRatio::new(16_777_217, 16_777_216)
                .unwrap()
                .binary32()
                .unwrap(),
            ExactRatio::whole(1)
        );
        assert_eq!(
            ExactRatio::new(16_777_219, 16_777_216)
                .unwrap()
                .binary32()
                .unwrap(),
            ExactRatio::new(4_194_305, 4_194_304).unwrap()
        );
        assert_eq!(
            ExactRatio::new(-16_777_217, 16_777_216)
                .unwrap()
                .binary32()
                .unwrap(),
            ExactRatio::whole(-1)
        );
        let (interval, multiplier) = periodic_interval(5, 666).unwrap();
        assert_eq!(interval, 3330);
        assert_eq!(multiplier.parts(), (1_745_879, 524_288));
        let heal = periodic_amount(ExactRatio::whole(100), multiplier, 500 * Q, false).unwrap();
        assert_eq!(heal.to_q().unwrap(), 332_999_992);
        assert_eq!(heal.trunc().unwrap(), 332);
        let damage = periodic_amount(
            ExactRatio::whole(10),
            periodic_interval(1, 666).unwrap().1,
            100 * Q,
            false,
        )
        .unwrap();
        let removed = nonlethal_periodic_damage(damage, 100).unwrap();
        assert_eq!((removed, 100 - removed), (6, 94));
        assert_eq!(nonlethal_periodic_damage(ExactRatio::whole(50), 10).unwrap(), 9);
        assert_eq!(nonlethal_periodic_damage(ExactRatio::whole(50), 1).unwrap(), 0);
    }

    #[test]
    fn periodic_cadence_never_accumulates_tick_rounding_drift() {
        for (ticks, expected) in [(1, [7, 14, 20]), (3, [20, 40, 60])] {
            let (interval, _) = periodic_interval(ticks, 666).unwrap();
            for (index, offset) in expected.into_iter().enumerate() {
                assert_eq!(
                    periodic_deadline(Tick(100), u64::try_from(index).unwrap() + 1, interval)
                        .unwrap(),
                    Tick(100 + offset)
                );
            }
        }
        assert!(periodic_interval(0, 666).is_err());
        assert!(periodic_interval(u32::MAX, 666).is_err());
        assert!(periodic_deadline(Tick(u64::MAX), 1, 666).is_err());
    }

    #[test]
    fn cancel_rate_preserves_both_source_casts_and_level_zero_branch() {
        assert_eq!(cancel_base_rate(25, 0, 0).unwrap(), 25);
        assert_eq!(cancel_base_rate(25, -20 * Q, 0).unwrap(), 20);
        assert_eq!(cancel_base_rate(25, 20 * Q, 0).unwrap(), 31);
        assert_eq!(cancel_buff_chance(20, 48, 1, 10, 90).unwrap(), 29);
        assert_eq!(cancel_buff_chance(25, 48, 0, 10, 90).unwrap(), 25);
        assert_eq!(cancel_buff_chance(25, 1, 85, 10, 90).unwrap(), 10);
        assert_eq!(cancel_buff_chance(200, 85, 1, 10, 90).unwrap(), 90);
        // Java division by zero becomes infinity/NaN, then int MAX/MIN/zero.
        assert_eq!(cancel_base_rate(25, 100 * Q, 0).unwrap(), i32::MAX);
        assert_eq!(cancel_base_rate(-25, 100 * Q, 0).unwrap(), i32::MIN);
        assert_eq!(cancel_base_rate(0, 100 * Q, 0).unwrap(), 0);
        assert_eq!(cancel_base_rate(25, 200 * Q, 0).unwrap(), -25);
        assert!(cancel_buff_chance(25, 1, 1, 90, 10).is_err());
    }

    #[test]
    fn affect_limit_requires_the_exact_rng_branch() {
        assert_eq!(affect_limit(42, 0, None).unwrap(), 0);
        assert_eq!(affect_limit(42, 1, Some(0)).unwrap(), 42);
        assert!(affect_limit(42, 0, Some(0)).is_err());
        assert!(affect_limit(42, 1, None).is_err());
        assert!(affect_limit(42, 1, Some(1)).is_err());
        assert!(affect_limit(u32::MAX, 2, Some(1)).is_err());
    }

    #[test]
    fn arithmetic_rounding_and_cross_cancellation_are_distinct() {
        let negative = ExactRatio::new(-3, 2).unwrap();
        assert_eq!(
            (negative.trunc().unwrap(), negative.floor().unwrap(), negative.round().unwrap()),
            (-1, -2, -1)
        );
        let a = ExactRatio::new(i128::MAX, 2).unwrap();
        assert_eq!(
            a.checked_mul(ExactRatio::new(2, i128::MAX).unwrap())
                .unwrap(),
            ExactRatio::whole(1)
        );
        assert!(a.checked_add(a).is_err());
        assert!(ExactRatio::new(1, 0).is_err());
        assert!(ExactRatio::new(i128::MIN, 1).is_err());
    }
}
