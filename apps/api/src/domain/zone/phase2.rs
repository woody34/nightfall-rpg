//! Phase 2 player identity and exact profession resource derivation. Legacy zones omit this block.
use super::scaled::{floor_div, mul, round_div, Q128};
use super::{resource_max, Fixed, Scaled, Speed, StatError, StatKind, StatRules, StatSheet};
use crate::domain::character_progression::{base_class_profile, CharacterIdentity, ClassState};
use crate::domain::class::{ClassId, ClassRegistry};
use crate::domain::subclass::Sex;
use crate::domain::Race;
use serde::{Deserialize, Serialize};

/// Private authoritative main-class state; never sent to observers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerProgression {
    /// Immutable account/race/appearance.
    pub identity: CharacterIdentity,
    /// Mutable main-class ledger, including frozen successful responses.
    pub class_state: ClassState,
    /// Derived reserved resource maximum; CP does not absorb damage.
    pub max_cp: u32,
}

/// Public player identity for AOI entry and reconnection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerIdentityView {
    /// Race.
    pub race: Race,
    /// Current profession, including valid id zero.
    pub class_id: ClassId,
    /// Immutable appearance.
    pub appearance: crate::domain::character_progression::CharacterAppearance,
}

/// Owner-only additions to resource updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)] // Mirrors the documented ClassState ledger and StatsChanged wire contract.
pub struct ClassStatsView {
    pub class_id: ClassId,
    pub sp: u64,
    pub cp: u32,
    pub max_cp: u32,
    pub token_tier_1_count: u32,
    pub token_tier_2_count: u32,
}

impl PlayerProgression {
    /// Public projection excludes account and receipts.
    pub fn public(&self) -> PlayerIdentityView {
        PlayerIdentityView {
            race: self.identity.race,
            class_id: self.class_state.current_class_id,
            appearance: self.identity.appearance,
        }
    }
    /// Owner resource projection.
    pub fn stats(&self) -> ClassStatsView {
        ClassStatsView {
            class_id: self.class_state.current_class_id,
            sp: self.class_state.sp,
            cp: self.class_state.cp,
            max_cp: self.max_cp,
            token_tier_1_count: self.class_state.token_tier_1_count,
            token_tier_2_count: self.class_state.token_tier_2_count,
        }
    }
}

/// Derives the existing combat sheet with exact current-profession growth and racial evasion.
pub fn profession_stats(
    rules: &StatRules,
    registry: &ClassRegistry,
    identity: &CharacterIdentity,
    state: &ClassState,
    level: u32,
) -> Result<(StatSheet, u32, Speed, Fixed), StatError> {
    let invalid = || StatError::Overflow;
    state.validate().map_err(|_| invalid())?;
    let class = registry.get(state.current_class_id).ok_or_else(invalid)?;
    if class.race != identity.race
        || class.base_class_id != identity.base_class_id
        || state.base_class_id != identity.base_class_id
        || class.tier > 2
    {
        return Err(invalid());
    }
    let root = rules
        .class(base_class_profile(identity.base_class_id).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    if root.base != class.base_stats {
        return Err(invalid());
    }
    let growth = registry.growth(class.id, level).map_err(|_| invalid())?;
    let con = rules.bonus().bonus(StatKind::Con, class.base_stats.con)?;
    let men = rules.bonus().bonus(StatKind::Men, class.base_stats.men)?;
    let mut values =
        StatSheet::for_player(rules, root, level, Some(rules.starter_weapon()))?.to_final();
    values.max_hp = resource_max(growth.hp.raw().into(), con)?;
    values.max_mp = resource_max(growth.mp.raw().into(), men)?;
    if identity.race == Race::Elf {
        let whole = round_div(mul(values.evasion.raw().into(), 103)?, mul(Q128, 100)?)?;
        values.evasion =
            Scaled::from_i128(mul(whole.min(rules.constants().evasion_cap.into()), Q128)?)?;
    }
    if values.max_hp == 0 {
        return Err(invalid());
    }
    let max_cp = resource_max(growth.cp.raw().into(), con)?;
    let run = u64::from(class.movement.run)
        .checked_add(if identity.race == Race::Elf { 3 } else { 0 })
        .ok_or_else(invalid)?;
    let speed = run.checked_mul(1000).ok_or_else(invalid)? / 320;
    let speed = Speed::from_milli_tiles_per_tick(u32::try_from(speed).map_err(|_| invalid())?);
    let radius = match identity.appearance.sex {
        Sex::Male => class.collision.radius_male,
        Sex::Female => class.collision.radius_female,
    };
    let radius = floor_div(mul(radius.raw().into(), 1000)?, mul(Q128, 32)?)?;
    let radius = Fixed::from_raw(i32::try_from(radius).map_err(|_| invalid())?);
    Ok((StatSheet::from_final(values)?, max_cp, speed, radius))
}

/// Transfer resource ratio, floored once and bounded; HP uses minimum one for living actors.
pub fn transfer_resource(current: u32, old_max: u32, new_max: u32, minimum: u32) -> u32 {
    let value = if old_max == 0 {
        0
    } else {
        u64::from(current)
            .saturating_mul(u64::from(new_max))
            .checked_div(u64::from(old_max))
            .unwrap_or(0)
    };
    u32::try_from(value)
        .unwrap_or(u32::MAX)
        .clamp(minimum.min(new_max), new_max)
}
