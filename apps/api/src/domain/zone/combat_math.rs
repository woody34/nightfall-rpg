//! Combat arithmetic (plan §3.1): hit and crit rolls, physical damage, attack timing in
//! ticks, hate and respawn values. Pure functions over [`StatSheet`]s and the formula
//! constants; the caller draws the random numbers in the plan's fixed order (§3.2) and
//! passes them in, so these functions never touch the zone's generator.

use super::entity::{Tick, TICK_MS};
use super::scaled::{
    add, ceil_div, floor_div, mul, narrow, round_div, sub, Scaled, StatError, Q128,
};
use super::stat_rules::FormulaConstants;
use super::stat_sheet::StatSheet;

/// Round accuracy/evasion separately as HF getters do, then
/// `chance‰ = clamp(800 + 20 * (round(acc) − round(eva)), 200, 980)`. Condition bonus
/// (position, terrain, weather) is neutral in Phase 1.
pub fn hit_chance_permille(
    c: &FormulaConstants,
    accuracy: Scaled,
    evasion: Scaled,
) -> Result<u32, StatError> {
    let base = mul(mul(c.hit_base.into(), c.hit_scale.into())?, Q128)?;
    let per_point = mul(c.hit_per_point.into(), c.hit_scale.into())?;
    let diff = mul(
        sub(round_div(accuracy.raw().into(), Q128)?, round_div(evasion.raw().into(), Q128)?)?,
        Q128,
    )?;
    let chance = floor_div(add(base, mul(per_point, diff)?)?, Q128)?;
    narrow(chance.clamp(c.hit_min_permille.into(), c.hit_max_permille.into()))
}

/// L2J `calcHitMiss` misses when `chance < roll`, so the attack **hits iff `roll <= chance`**
/// with `roll` uniform in `0..roll_range`: a nominal 80 % is 801 of 1000 outcomes.
pub fn hit_lands(c: &FormulaConstants, chance_permille: u32, roll: u32) -> Result<bool, StatError> {
    if roll >= c.hit_roll_range {
        return Err(StatError::DrawOutOfRange(roll.into()));
    }
    Ok(roll <= chance_permille)
}

/// L2J `calcCrit` crits when `rate > roll`: **crit iff `roll < crit‰`**, `roll` in
/// `0..roll_range`.
pub fn crit_lands(c: &FormulaConstants, crit_permille: u32, roll: u32) -> Result<bool, StatError> {
    if roll >= c.crit_roll_range {
        return Err(StatError::DrawOutOfRange(roll.into()));
    }
    Ok(roll < crit_permille)
}

/// Physical normal-attack damage of a landed hit (HF `calcPhysDam`):
/// `max(1, F(K * A_Q * critFactor * (div + j) / (D_Q * div)))`, `K` = 76, `critFactor` 2 on
/// a crit else 1, `div` = 100, and `spread` = `j` drawn uniformly from
/// `-r..=r` with `r = attacker.random_damage()`. Soulshots, position, traits, attributes,
/// `PvP` and level penalties are neutral in Phase 1.
pub fn physical_damage(
    c: &FormulaConstants,
    attacker: &StatSheet,
    target: &StatSheet,
    crit: bool,
    spread: i64,
) -> Result<u32, StatError> {
    physical_damage_with_critical_bonus(c, attacker, target, crit, spread, 100)
}

/// Critical-only racial percentage, included before the damage's single final floor.
pub(super) fn physical_damage_with_critical_bonus(
    c: &FormulaConstants,
    attacker: &StatSheet,
    target: &StatSheet,
    crit: bool,
    spread: i64,
    critical_percent: u32,
) -> Result<u32, StatError> {
    let radius = i64::from(attacker.random_damage());
    if spread < radius.saturating_neg() || spread > radius {
        return Err(StatError::DrawOutOfRange(spread));
    }
    let crit_factor: i128 = if crit { c.crit_multiplier.into() } else { 1 };
    let divisor: i128 = c.random_divisor.into();
    let numerator = mul(
        mul(mul(c.damage_coefficient.into(), attacker.p_atk().raw().into())?, crit_factor)?,
        add(divisor, spread.into())?,
    )?;
    let denominator = mul(target.p_def().raw().into(), divisor)?;
    let numerator = mul(numerator, if crit { critical_percent.into() } else { 100 })?;
    let denominator = mul(denominator, 100)?;
    narrow(floor_div(numerator, denominator)?.max(1))
}

/// When a swing lands and when the next may start, in whole ticks from the swing's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackTiming {
    /// Source interval in integer ms, retained as `(ms, 1)` for display compatibility.
    pub interval_ms: (i128, i128),
    /// Ticks to impact: ceiling of the source integer half-interval over `TICK_MS`.
    pub impact_ticks: u64,
    /// Ticks to next swing: ceiling of the source integer interval over `TICK_MS`.
    pub cycle_ticks: u64,
}

/// Attack timing from `speed_Q` (HF `calculateTimeBetweenAttacks` = 500000 / pAtkSpd,
/// impact at half). Match HF integer millisecond casts before ceiling to ticks.
pub fn attack_timing(c: &FormulaConstants, speed: Scaled) -> Result<AttackTiming, StatError> {
    let speed = i128::from(speed.raw());
    if speed <= 0 {
        return Err(StatError::NonPositiveDivisor);
    }
    // HF rounds speed, truncates the interval to integer ms, then halves that integer.
    // Quantize those source deadlines to ticks; do not skip the millisecond casts.
    let speed = round_div(speed, Q128)?;
    let interval = floor_div(c.attack_interval_ms.into(), speed)?;
    let impact_ms = floor_div(interval, c.impact_divisor.into())?;
    let cycle = ceil_div(interval, TICK_MS.into())?.max(1);
    let impact = ceil_div(impact_ms, TICK_MS.into())?.max(1);
    Ok(AttackTiming {
        interval_ms: (interval, 1),
        impact_ticks: narrow(impact)?,
        cycle_ticks: narrow(cycle)?,
    })
}

/// Hate for `damage` landed on an NPC of `npc_level`: `F(d * 100 / (npcLevel + 7))`.
pub fn damage_hate(c: &FormulaConstants, damage: u32, npc_level: u32) -> Result<u64, StatError> {
    let numerator = mul(damage.into(), c.hate_numerator.into())?;
    let level = add(npc_level.into(), c.hate_level_offset.into())?;
    narrow(floor_div(numerator, level)?)
}

/// `current + added`, capped at 999,999,999 (HF `AggroInfo.addHate`).
#[must_use]
pub fn add_hate(c: &FormulaConstants, current: u64, added: u64) -> u64 {
    current.saturating_add(added).min(c.hate_cap)
}

/// Ticks per second of simulation time.
const TICKS_PER_SECOND: u64 = 1000 / TICK_MS;

/// NPC respawn tick: `deathTick + 10 * (delaySeconds + jitter)`, `jitter` drawn uniformly
/// from `0..=random_seconds` (inclusive) by the caller.
pub fn npc_respawn_tick(
    death: Tick,
    delay_seconds: u32,
    random_seconds: u32,
    jitter: u32,
) -> Result<Tick, StatError> {
    if jitter > random_seconds {
        return Err(StatError::DrawOutOfRange(jitter.into()));
    }
    let seconds = u64::from(delay_seconds)
        .checked_add(jitter.into())
        .ok_or(StatError::Overflow)?;
    let ticks = seconds
        .checked_mul(TICKS_PER_SECOND)
        .ok_or(StatError::Overflow)?;
    death
        .0
        .checked_add(ticks)
        .map(Tick)
        .ok_or(StatError::Overflow)
}

/// HP and MP after a town respawn: `HP = max(1, F(maxHP * 0.65))`, `MP = F(maxMP * 0)`.
pub fn town_respawn_vitals(
    c: &FormulaConstants,
    sheet: &StatSheet,
) -> Result<(u32, u32), StatError> {
    let hp =
        floor_div(mul(sheet.max_hp().into(), c.respawn_restore_hp.raw().into())?, Q128)?.max(1);
    let mp = floor_div(mul(sheet.max_mp().into(), c.respawn_restore_mp.raw().into())?, Q128)?;
    Ok((narrow(hp)?, narrow(mp)?))
}

/// Spawn protection after a town respawn, in ticks (HF `PlayerSpawnProtection` is in
/// seconds: 600 s = 6000 ticks). Attacking ends it early.
pub fn spawn_protection_ticks(c: &FormulaConstants) -> Result<u64, StatError> {
    u64::from(c.spawn_protection_seconds)
        .checked_mul(TICKS_PER_SECOND)
        .ok_or(StatError::Overflow)
}
