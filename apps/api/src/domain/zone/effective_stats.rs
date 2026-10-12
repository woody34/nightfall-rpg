//! Cold reconstruction of Phase 3 stat calculators and getter projections. Inputs are raw
//! template/weapon/growth values, never the already rounded legacy profession sheet.
//! Source: pinned CharStat.java:95-113,143-149,171-177,202-227,252-275,401-404;
//! PcStat.java:186-194,233-241. Rebuilding never runs onStart, heals, or consumes RNG.

use std::collections::BTreeMap;

use super::effects::{continuous_handler, StatusFlags};
use super::scaled::{add, mul, narrow, round_div};
use super::skill::{SkillDef, SkillHandler, SkillRef};
use super::skill_math::ExactRatio;
use super::stat_functions::{
    compile_maximum_hook, compile_modifier, FunctionConditions, StatCalculator, StatFunction,
};
use super::{
    FinalStats, Phase3Entity, Phase3Rules, RestoredModifierSource, Scaled, StatError, StatSheet, Q,
};

/// All raw getter inputs must come from immutable validated profiles. Zero is a meaningful
/// input for accuracy/evasion; no missing profile is filled by a guessed default.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct EffectiveStatBases {
    pub level: u32,
    pub player: bool,
    pub max_hp_q: i64,
    pub max_mp_q: i64,
    pub max_cp_q: i64,
    pub p_atk_q: i64,
    pub p_def_q: i64,
    pub m_atk_q: i64,
    pub m_def_q: i64,
    pub attack_speed_q: i64,
    pub cast_speed_q: i64,
    pub run_speed_q: i64,
    pub accuracy_q: i64,
    pub evasion_q: i64,
    pub crit_q: i64,
    pub magic_crit_q: i64,
    pub random_damage: u32,
    /// Existing Elf adjustment is applied once after getter rounding, as in Phase2.
    pub elf: bool,
    /// Source configured run-speed boost (PcStat.java:187), before the player cap.
    pub run_boost_q: i64,
}

/// Remaining source caps supplied explicitly by the immutable stat/profile bundle.
/// C5 attack/cast speed and evasion caps are always read from `Phase3Rules` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct EffectiveStatLimits {
    pub physical_crit: u32,
    pub magic_crit: u32,
    pub run_speed_q: i64,
}

/// Fully assembled player getter inputs. This contains raw bases and declarative Funcs,
/// not a previously rounded physical sheet; it is reconstructed rather than persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct PlayerStatInputs {
    pub bases: EffectiveStatBases,
    pub functions: Vec<StatFunction>,
    pub limits: EffectiveStatLimits,
}
/// V6 canonical equipment and its separately hash-bound catalogue.
#[derive(Clone, Copy)]
pub struct EquipmentStatContext<'a> {
    /// Retained instances and provider installation order.
    pub player: &'a super::Phase4Player,
    /// Immutable item and class-slot definitions.
    pub rules: &'a super::Phase4Rules,
}
impl<'a> EquipmentStatContext<'a> {
    /// Actual weapon kind; an empty right hand uses class fist bases.
    pub fn weapon_kind(self) -> Result<&'a str, StatError> {
        let Some(id) = self.player.paperdoll.get(&super::EquipSlot::Rhand) else {
            return Ok("FIST");
        };
        let item = self.player.inventory.get(id).ok_or(StatError::Overflow)?;
        let definition = self
            .rules
            .parts()
            .items
            .get(&item.template_id)
            .ok_or(StatError::Overflow)?;
        Ok(&definition.weapon_type)
    }
}
impl PlayerStatInputs {
    /// Build V6 from actual item providers and hash-bound class defence slots. Legacy
    /// construction remains unchanged; equipment removal returns to class fist bases.
    pub fn from_equipment(
        stats: &super::StatRules,
        classes: &crate::domain::class::ClassRegistry,
        rules: &Phase3Rules,
        class_id: crate::domain::class::ClassId,
        level: u32,
        equipment: EquipmentStatContext<'_>,
    ) -> Result<Self, StatError> {
        use super::item_functions::{equipped_functions, subtract_defence_slots};
        super::item_equipment::validate_paperdoll(
            &equipment.player.inventory,
            &equipment.player.paperdoll,
            equipment.rules,
        )
        .map_err(|_| StatError::Overflow)?;
        let mut inputs = Self::from_class(stats, classes, rules, class_id, level)?;
        let class = classes.get(class_id).ok_or(StatError::Overflow)?;
        let weapon = equipment
            .player
            .paperdoll
            .get(&super::EquipSlot::Rhand)
            .map(|id| {
                equipment
                    .player
                    .inventory
                    .get(id)
                    .ok_or(StatError::Overflow)
            })
            .transpose()?;
        inputs.bases.random_damage = match weapon {
            Some(item) => {
                equipment
                    .rules
                    .parts()
                    .items
                    .get(&item.template_id)
                    .ok_or(StatError::Overflow)?
                    .random_damage
            },
            None => super::fist_random_damage(stats.constants(), level)?,
        };
        inputs.functions = super::stat_functions::standard_functions(
            &class.base_stats,
            stats.bonus(),
            super::level_mod(stats.constants(), level)?,
            level,
            true,
            weapon.is_some(),
        )?;
        let slots = equipment
            .rules
            .parts()
            .class_slots
            .iter()
            .find(|row| row.base_class_id == class.base_class_id.0)
            .ok_or(StatError::Overflow)?;
        subtract_defence_slots(&mut inputs.functions, equipment.player, equipment.rules, slots)?;
        inputs
            .functions
            .extend(equipped_functions(equipment.player, equipment.rules)?);
        Ok(inputs)
    }

    /// Build from the current profession's growth and source templates plus the actually
    /// equipped starter weapon. Order0 weapon SET replaces class6 with weapon5 for M.Atk.
    pub fn from_class(
        stats: &super::StatRules,
        classes: &crate::domain::class::ClassRegistry,
        rules: &Phase3Rules,
        class_id: crate::domain::class::ClassId,
        level: u32,
    ) -> Result<Self, StatError> {
        use crate::domain::character_progression::base_class_profile;
        let class = classes.get(class_id).ok_or(StatError::Overflow)?;
        let root = stats
            .class(base_class_profile(class.base_class_id).ok_or(StatError::Overflow)?)
            .ok_or(StatError::Overflow)?;
        let profile = rules
            .profiles()
            .classes
            .get(&class_id)
            .ok_or(StatError::Overflow)?;
        if class.tier > 2
            || class.base_stats != root.base
            || profile.base_stats.values() != class.base_stats
        {
            return Err(StatError::Overflow);
        }
        let growth = classes
            .growth(class_id, level)
            .map_err(|_| StatError::Overflow)?;
        let weapon = &rules.profiles().starter_weapon;
        weapon.validate_physical(stats.starter_weapon())?;
        let constants = &rules.profiles().limits.constants;
        let q = |n: u32| super::Scaled::from_int(i64::from(n)).map(super::Scaled::raw);
        let bases = EffectiveStatBases {
            level,
            player: true,
            max_hp_q: growth.hp.raw(),
            max_mp_q: growth.mp.raw(),
            max_cp_q: growth.cp.raw(),
            p_atk_q: root.fist_p_atk.raw(),
            p_def_q: root.p_def_unarmoured.raw(),
            m_atk_q: q(profile.magical.base_m_atk)?,
            m_def_q: q(profile.magical.base_m_def)?,
            attack_speed_q: q(root.fist_attack_speed)?,
            cast_speed_q: q(profile.magical.base_m_atk_speed)?,
            run_speed_q: q(class.movement.run)?,
            accuracy_q: q(profile.combat_getter_bases.accuracy)?,
            evasion_q: q(profile.combat_getter_bases.evasion)?,
            crit_q: q(root.fist_crit_rate)?,
            magic_crit_q: q(profile.combat_getter_bases.magic_critical)?,
            random_damage: weapon.random_damage,
            elf: class.race == crate::domain::Race::Elf,
            run_boost_q: q(constants.run_speed_boost.value)?,
        };
        let mut functions = super::stat_functions::standard_functions(
            &class.base_stats,
            stats.bonus(),
            super::level_mod(stats.constants(), level)?,
            level,
            true,
            true,
        )?;
        functions.extend(weapon.functions());
        Ok(Self {
            bases,
            functions,
            limits: EffectiveStatLimits {
                physical_crit: stats.constants().crit_cap_permille,
                magic_crit: constants.max_magic_crit.value,
                run_speed_q: q(constants.max_run_speed.value)?,
            },
        })
    }
    /// Fold canonical passive/effect sources without running one-time hooks or healing.
    pub fn derive(
        &self,
        entity: &Phase3Entity,
        rules: &Phase3Rules,
        can_resurrect: bool,
    ) -> Result<EffectiveStats, StatError> {
        derive_effective_stats(
            &self.bases,
            self.functions.clone(),
            entity,
            rules,
            self.limits,
            FunctionConditions {
                weapon_kind: &rules.profiles().starter_weapon.weapon_type,
                can_resurrect,
            },
        )
    }
}

/// Cold NPC calculator inputs. Physical template values are already the tuned unmodified
/// bases: never apply player CON/STR/DEX/level multipliers to them a second time. Magic,
/// accuracy/evasion/critical getters still execute their pinned builtins.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct NpcStatInputs {
    pub bases: EffectiveStatBases,
    pub functions: Vec<StatFunction>,
    pub limits: EffectiveStatLimits,
}
impl NpcStatInputs {
    /// Reconstruct from the rules' canonical original template, not `CombatState.sheet`.
    /// `CharStat.java:270` initializes magic critical to1; the NPC builtin and getter each
    /// apply their separate x10 factors (`FuncMAtkCritical.java:42-44`).
    pub fn from_profile(
        stats: &super::StatRules,
        rules: &Phase3Rules,
        template: &str,
    ) -> Result<Self, StatError> {
        let profile = rules
            .profiles()
            .npcs
            .get(template)
            .ok_or(StatError::Overflow)?;
        profile
            .physical
            .validate()
            .map_err(|_| StatError::Overflow)?;
        let physical = &profile.physical;
        let magic = &profile.template.template;
        let base = profile.template.base_stats.values();
        let constants = &rules.profiles().limits.constants;
        let q = |n: u32| Scaled::from_int(i64::from(n)).map(Scaled::raw);
        // Original milli-tiles/tick -> L2 units/second, retaining Q precision. The inverse
        // EffectiveStats::movement_speed roundtrip is exact before any speed modifiers.
        let run_speed_q =
            narrow(mul(i128::from(physical.move_speed.milli_tiles_per_tick()), 320_000)?)?;
        let mut functions = super::stat_functions::standard_functions(
            &base,
            stats.bonus(),
            super::level_mod(stats.constants(), physical.level)?,
            physical.level,
            false,
            false,
        )?;
        functions.retain(|f| {
            matches!(
                f.key.stat.as_str(),
                "mAtk" | "mDef" | "mAtkSpd" | "mCritRate" | "accCombat" | "rEvas" | "critRate"
            )
        });
        Ok(Self {
            bases: EffectiveStatBases {
                level: physical.level,
                player: false,
                max_hp_q: q(physical.max_hp)?,
                max_mp_q: q(physical.max_mp)?,
                max_cp_q: 0,
                p_atk_q: physical.p_atk_q,
                p_def_q: physical.p_def_q,
                m_atk_q: q(magic.m_atk)?,
                m_def_q: q(magic.m_def)?,
                attack_speed_q: q(physical.attack_speed)?,
                cast_speed_q: q(magic.m_atk_speed)?,
                run_speed_q,
                accuracy_q: q(profile.template.combat_getter_bases.accuracy)?,
                evasion_q: q(profile.template.combat_getter_bases.evasion)?,
                crit_q: q(super::NPC_BASE_CRIT)?,
                magic_crit_q: Q,
                random_damage: super::fist_random_damage(stats.constants(), physical.level)?,
                elf: false,
                run_boost_q: 0,
            },
            functions,
            limits: EffectiveStatLimits {
                physical_crit: stats.constants().crit_cap_permille,
                magic_crit: constants.max_magic_crit.value,
                run_speed_q: q(constants.max_run_speed.value)?,
            },
        })
    }
    /// Install canonical timed sources without callback/RNG/resource mutation. NPCs have
    /// no learned passive, player CP, or Death Penalty ledger.
    pub fn derive(
        &self,
        entity: &Phase3Entity,
        rules: &Phase3Rules,
    ) -> Result<EffectiveStats, StatError> {
        if entity.player.is_some() || entity.fractions.cp != 0 {
            return Err(StatError::Overflow);
        }
        derive_effective_stats(
            &self.bases,
            self.functions.clone(),
            entity,
            rules,
            self.limits,
            FunctionConditions {
                weapon_kind: "NONE",
                can_resurrect: false,
            },
        )
    }
}

/// Derived trait state; counts distinguish no provider from a neutral-valued provider.
/// Defence immunity is independent of a multiplicative provider (DefenceTrait.java:81-91).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
pub struct EffectiveTrait {
    pub attack: ExactRatio,
    pub attack_providers: u32,
    pub defence: ExactRatio,
    pub defence_providers: u32,
    pub immunity_providers: u32,
}
impl Default for EffectiveTrait {
    fn default() -> Self {
        Self {
            attack: ExactRatio::whole(1),
            attack_providers: 0,
            defence: ExactRatio::whole(1),
            defence_providers: 0,
            immunity_providers: 0,
        }
    }
}

/// Rebuildable cache. No member is separately serialized or hashed as authority; the
/// physical projection is compared with CombatState.sheet when restoring live snapshots.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
pub struct EffectiveStats {
    pub physical: StatSheet,
    pub max_cp: u32,
    pub m_atk_q: i64,
    pub m_def_q: i64,
    pub cast_speed: u32,
    pub run_speed_q: i64,
    pub magic_crit: u32,
    pub recoverable_hp: u32,
    pub recoverable_mp: u32,
    pub recoverable_cp: u32,
    pub flags: StatusFlags,
    pub traits: BTreeMap<String, EffectiveTrait>,
    /// Retained for context-specific cost/reuse/regen/trait queries at the next read.
    pub calculator: StatCalculator,
}

impl EffectiveStats {
    /// Source L2 units/second to zone milli-tiles/100ms, with one final floor. The current
    /// effective speed changes movement; it never rescales existing swing/cast deadlines.
    pub fn movement_speed(&self) -> Result<super::Speed, StatError> {
        if self.run_speed_q < 0 {
            return Err(StatError::Overflow);
        }
        let raw = mul(i128::from(self.run_speed_q), 1000)?
            .checked_div(mul(i128::from(Q), 320)?)
            .ok_or(StatError::Overflow)?;
        Ok(super::Speed::from_milli_tiles_per_tick(narrow(raw)?))
    }
}

/// Compile installed sources in shared sequence order. Each active handler was selected
/// at admission/onStart; it is not reconsidered or replayed during cache reconstruction.
/// Passive attach conditions are evaluated against current equipment and life eligibility.
fn modifiers(
    entity: &Phase3Entity,
    rules: &Phase3Rules,
    conditions: FunctionConditions<'_>,
    functions: &mut Vec<StatFunction>,
) -> Result<BTreeMap<String, EffectiveTrait>, StatError> {
    let mut traits = BTreeMap::new();
    for (sequence, source) in &entity.modifier_install_order {
        let (reference, selected) = match source {
            RestoredModifierSource::Passive(skill) => (*skill, None),
            RestoredModifierSource::Effect(id) => {
                let effect = entity
                    .effects
                    .entries()
                    .get(id)
                    .ok_or(StatError::Overflow)?;
                (effect.skill, Some(effect.handlers.as_slice()))
            },
            RestoredModifierSource::DeathPenalty => (
                SkillRef {
                    id: super::skill::SkillId(5076),
                    rank: u32::from(
                        entity
                            .player
                            .as_ref()
                            .ok_or(StatError::Overflow)?
                            .death_penalty_level,
                    ),
                },
                None,
            ),
        };
        let skill = rules
            .skills()
            .get(reference.id)
            .ok_or(StatError::Overflow)?;
        if *sequence == 0
            || reference.rank == 0
            || reference.rank > skill.max_level
            || skill.runtime_status != crate::domain::class::RuntimeSkillStatus::Executable
        {
            return Err(StatError::Overflow);
        }
        for (index, handler) in skill.effects.iter().enumerate() {
            let ordinal = u32::try_from(index).map_err(|_| StatError::Overflow)?;
            if !continuous_handler(&handler.name) {
                continue;
            }
            if let Some(selected) = selected {
                if !selected.iter().any(|h| h.xml_ordinal == ordinal) {
                    continue;
                }
            } else {
                let mut eligible = true;
                for condition in skill
                    .conditions
                    .iter()
                    .chain(&handler.section_conditions)
                    .chain(&handler.conditions)
                {
                    if !conditions.matches(condition)? {
                        eligible = false;
                        break;
                    }
                }
                if !eligible {
                    continue;
                }
            }
            compile_handler(functions, handler, skill, reference.rank, *sequence, ordinal)?;
            fold_traits(&mut traits, handler, reference.rank, skill.max_level)?;
        }
    }
    Ok(traits)
}

fn compile_handler(
    functions: &mut Vec<StatFunction>,
    handler: &SkillHandler,
    skill: &SkillDef,
    rank: u32,
    sequence: u64,
    ordinal: u32,
) -> Result<(), StatError> {
    if let Some(function) = compile_maximum_hook(handler, rank, skill.max_level, sequence, ordinal)?
    {
        functions.push(function);
    }
    for (index, modifier) in handler.stat_children.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| StatError::Overflow)?
            .checked_add(1)
            .ok_or(StatError::Overflow)?;
        functions.push(compile_modifier(
            modifier,
            rank,
            skill.max_level,
            sequence,
            ordinal,
            index,
        )?);
    }
    Ok(())
}

fn fold_traits(
    traits: &mut BTreeMap<String, EffectiveTrait>,
    handler: &SkillHandler,
    rank: u32,
    max_rank: u32,
) -> Result<(), StatError> {
    let defence = match handler.name.as_str() {
        "DefenceTrait" => true,
        "AttackTrait" => false,
        _ => return Ok(()),
    };
    for (name, values) in &handler.params {
        let raw = ExactRatio::parse_literal(values.at(rank, max_rank).ok_or(StatError::Overflow)?)?
            .binary32()?;
        if defence && raw == ExactRatio::whole(0) {
            continue;
        }
        // Each arithmetic operation and each array product rounds as the source's 24-bit
        // significand. AttackTrait.java:52; DefenceTrait.java:56-61; CharStat.java:42-51.
        let factor = raw
            .checked_add(ExactRatio::whole(100))?
            .binary32()?
            .checked_div(ExactRatio::whole(100))?
            .binary32()?;
        let entry = traits.entry(name.clone()).or_default();
        if defence && factor.checked_cmp(ExactRatio::whole(2))?.is_ge() {
            entry.immunity_providers = entry
                .immunity_providers
                .checked_add(1)
                .ok_or(StatError::Overflow)?;
        } else if defence {
            entry.defence = entry.defence.checked_mul(factor)?.binary32()?;
            entry.defence_providers = entry
                .defence_providers
                .checked_add(1)
                .ok_or(StatError::Overflow)?;
        } else {
            entry.attack = entry.attack.checked_mul(factor)?.binary32()?;
            entry.attack_providers = entry
                .attack_providers
                .checked_add(1)
                .ok_or(StatError::Overflow)?;
        }
    }
    Ok(())
}

/// Exact getter boundaries after ordered builtin/passive/effect functions. Caller supplies
/// builtins reconstructed from raw class/base attributes; no old rounded sheet is accepted.
/// `limits` also comes from validated source data, not client input.
#[allow(clippy::similar_names)] // HP/MP/CP are the source and domain resource names.
pub fn derive_effective_stats(
    base: &EffectiveStatBases,
    builtins: impl IntoIterator<Item = StatFunction>,
    entity: &Phase3Entity,
    rules: &Phase3Rules,
    limits: EffectiveStatLimits,
    conditions: FunctionConditions<'_>,
) -> Result<EffectiveStats, StatError> {
    let mut functions: Vec<_> = builtins.into_iter().collect();
    let traits = modifiers(entity, rules, conditions, &mut functions)?;
    let calculator = StatCalculator::try_new(functions)?;
    let fold = |stat: &str, initial: i64| calculator.fold(stat, initial, conditions);
    // CharStat.calcStat only clamps <=0, not every positive fraction below one.
    let positive = |stat: &str, initial: i64| -> Result<i64, StatError> {
        let value = fold(stat, initial)?;
        Ok(if value <= 0 { Q } else { value })
    };
    let trunc = |q: i64| -> Result<u32, StatError> {
        narrow(
            i128::from(q)
                .checked_div(i128::from(Q))
                .ok_or(StatError::Overflow)?,
        )
    };
    let rounded =
        |q: i64| -> Result<u32, StatError> { narrow(round_div(i128::from(q), i128::from(Q))?) };
    let max_hp = trunc(positive("maxHp", base.max_hp_q)?)?;
    let max_mp = trunc(positive("maxMp", base.max_mp_q)?)?;
    let max_cp = if base.player {
        trunc(positive("maxCp", base.max_cp_q)?)?
    } else {
        0
    };
    if max_hp == 0 || !(1..=85).contains(&base.level) || limits.run_speed_q <= 0 {
        return Err(StatError::Overflow);
    }
    let mut evasion =
        rounded(fold("rEvas", base.evasion_q)?)?.min(rules.constants().max_evasion.value);
    if base.elf {
        evasion = narrow(round_div(mul(i128::from(evasion), 103)?, 100)?)?;
        evasion = evasion.min(rules.constants().max_evasion.value);
    }
    let attack_speed = rounded(positive("pAtkSpd", base.attack_speed_q)?)?;
    let attack_speed = if base.player {
        attack_speed.min(rules.constants().max_physical_attack_speed.value)
    } else {
        attack_speed
    };
    let cast_speed = trunc(positive("mAtkSpd", base.cast_speed_q)?)?
        .min(rules.constants().max_magic_attack_speed.value);
    let run = if base.run_speed_q <= 0 {
        0
    } else {
        fold("runSpd", base.run_speed_q)?
    };
    let run = if base.elf {
        narrow(add(i128::from(run), mul(3, i128::from(Q))?)?)?
    } else {
        run
    };
    let run_speed_q = if base.player {
        narrow::<i64>(add(i128::from(run), i128::from(base.run_boost_q))?)?.min(limits.run_speed_q)
    } else {
        run
    };
    let physical = StatSheet::from_final(FinalStats {
        level: base.level,
        max_hp,
        max_mp,
        p_atk: Scaled::from_raw(positive("pAtk", base.p_atk_q)?),
        p_def: Scaled::from_raw(positive("pDef", base.p_def_q)?),
        accuracy: Scaled::from_int(i64::from(rounded(fold("accCombat", base.accuracy_q)?)?))?,
        evasion: Scaled::from_int(i64::from(evasion))?,
        crit_permille: trunc(fold("critRate", base.crit_q)?)?.min(limits.physical_crit),
        attack_speed: Scaled::from_int(i64::from(attack_speed))?,
        random_damage: base.random_damage,
    })?;
    Ok(EffectiveStats {
        physical,
        max_cp,
        m_atk_q: positive("mAtk", base.m_atk_q)?,
        m_def_q: positive("mDef", base.m_def_q)?,
        cast_speed,
        run_speed_q,
        magic_crit: trunc(fold("mCritRate", base.magic_crit_q)?)?
            .checked_mul(10)
            .ok_or(StatError::Overflow)?
            .min(limits.magic_crit),
        recoverable_hp: trunc(fold(
            "maxRecoverableHp",
            Scaled::from_int(i64::from(max_hp))?.raw(),
        )?)?,
        recoverable_mp: trunc(fold(
            "maxRecoverableMp",
            Scaled::from_int(i64::from(max_mp))?.raw(),
        )?)?,
        recoverable_cp: if base.player {
            trunc(fold("maxRecoverableCp", Scaled::from_int(i64::from(max_cp))?.raw())?)?
        } else {
            0
        },
        flags: entity
            .effects
            .status_flags(rules.skills())
            .map_err(|_| StatError::Overflow)?,
        traits,
        calculator,
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
    use crate::domain::zone::phase3_entity::tests::{blank, source_effect};
    use crate::domain::zone::stat_functions::{standard_functions, FunctionKey, FunctionOp};
    use crate::domain::zone::{
        effects::EffectList, skill::SkillId, skill_math, state_regen::*, StatKind,
    };
    use crate::infrastructure::{
        rules_data::{load_rules, RulesSource},
        skill_data::{load_phase3_rules, Phase3DataSource},
    };
    const CONDITIONS: FunctionConditions<'static> = FunctionConditions {
        weapon_kind: "SWORD",
        can_resurrect: false,
    };
    const LIMITS: EffectiveStatLimits = EffectiveStatLimits {
        physical_crit: 500,
        magic_crit: 200,
        run_speed_q: 250 * Q,
    };
    fn bases() -> EffectiveStatBases {
        EffectiveStatBases {
            level: 1,
            player: true,
            max_hp_q: 100 * Q,
            max_mp_q: 100 * Q,
            max_cp_q: 100 * Q,
            p_atk_q: Q,
            p_def_q: Q,
            m_atk_q: Q,
            m_def_q: Q,
            attack_speed_q: 300 * Q,
            cast_speed_q: 333 * Q,
            run_speed_q: 100 * Q,
            accuracy_q: 0,
            evasion_q: 0,
            crit_q: 4 * Q,
            magic_crit_q: Q,
            random_damage: 0,
            elf: false,
            run_boost_q: 0,
        }
    }
    fn function(stat: &str, operation: FunctionOp) -> StatFunction {
        StatFunction {
            key: FunctionKey {
                stat: stat.into(),
                priority: 1,
                install_sequence: 0,
                handler_ordinal: 0,
                function_ordinal: 0,
            },
            operation,
            conditions: vec![],
        }
    }
    #[test]
    fn npc_cold_rebuild_uses_original_tuned_bases_and_source_magic_getters() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let inputs = NpcStatInputs::from_profile(&stats, &rules, "keltir").unwrap();
        let mut npc = blank();
        npc.player = None;
        let original = inputs.derive(&npc, &rules).unwrap();
        assert_eq!(original.physical.max_hp(), 44);
        assert_eq!(original.physical.max_mp(), 20);
        assert_eq!(original.physical.p_atk().raw(), 8_800_000);
        assert_eq!(original.physical.p_def().raw(), 26_900_000);
        assert_eq!(original.physical.attack_speed().raw(), 253 * Q);
        assert_eq!(original.movement_speed().unwrap().milli_tiles_per_tick(), 400);
        assert_eq!(original.max_cp, 0);
        let base = rules.profiles().npcs["keltir"].template.base_stats.values();
        let lm = super::super::level_mod(stats.constants(), 1).unwrap();
        let ratio = |q: i64| ExactRatio::new(i128::from(q), i128::from(Q)).unwrap();
        let bonus =
            |kind: StatKind| ratio(stats.bonus().bonus(kind, kind.of(&base)).unwrap().raw());
        assert_eq!(
            original.m_atk_q,
            skill_math::magic_attack_mod(
                ExactRatio::whole(5),
                bonus(StatKind::Int),
                ratio(lm.raw())
            )
            .unwrap()
            .to_q()
            .unwrap()
        );
        assert_eq!(
            original.m_def_q,
            skill_math::magic_defence_mod(
                ExactRatio::whole(29),
                bonus(StatKind::Men),
                ratio(lm.raw())
            )
            .unwrap()
            .to_q()
            .unwrap()
        );
        let effect = source_effect(&rules, 1, 1040);
        let id = effect.id;
        npc.effects = EffectList::from_entries([(id, effect)].into()).unwrap();
        npc.modifier_install_order
            .insert(1, RestoredModifierSource::Effect(id));
        let buffed = inputs.derive(&npc, &rules).unwrap();
        assert_eq!(buffed.physical.p_def().raw(), 29_052_000); //26.9*1.08, once.
        let decoded: Phase3Entity =
            serde_json::from_slice(&serde_json::to_vec(&npc).unwrap()).unwrap();
        assert_eq!(inputs.derive(&decoded, &rules).unwrap(), buffed);
        assert_eq!(inputs.derive(&npc, &rules).unwrap(), buffed);
        npc.effects.remove(id);
        npc.modifier_install_order.clear();
        assert_eq!(inputs.derive(&npc, &rules).unwrap(), original);
        assert!(NpcStatInputs::from_profile(&stats, &rules, "missing").is_err());
        assert!(inputs.derive(&blank(), &rules).is_err());
    }
    #[test]
    fn cold_effect_reconstruction_uses_source_modifiers_without_replaying_hooks() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let mut entity = blank();
        let effect = source_effect(&rules, 1, 110);
        let id = effect.id;
        entity.effects = EffectList::from_entries(BTreeMap::from([(id, effect)])).unwrap();
        entity
            .modifier_install_order
            .insert(1, RestoredModifierSource::Effect(id));
        let before = entity.clone();
        let first =
            derive_effective_stats(&bases(), [], &entity, &rules, LIMITS, CONDITIONS).unwrap();
        assert_eq!(first.physical.p_def().raw(), 1801 * Q);
        assert_eq!(first.m_def_q, 1621 * Q);
        assert!(first.flags.intersects(StatusFlags::ROOT));
        assert_eq!(first.calculator.fold("cancelVuln", Q, CONDITIONS).unwrap(), Q / 5);
        assert_eq!(
            first,
            derive_effective_stats(&bases(), [], &entity, &rules, LIMITS, CONDITIONS).unwrap()
        );
        assert_eq!(entity, before);
        entity.effects.remove(id);
        entity.modifier_install_order.clear();
        let removed =
            derive_effective_stats(&bases(), [], &entity, &rules, LIMITS, CONDITIONS).unwrap();
        assert_eq!(removed.physical.p_def().raw(), Q);
        assert!(!removed.flags.intersects(StatusFlags::ROOT));
    }
    #[test]
    fn getter_rounding_and_source_caps_are_distinct() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let mut base = bases();
        base.max_hp_q = 100 * Q + 999_999;
        base.cast_speed_q = 1999 * Q + 999_999;
        base.attack_speed_q = 1499 * Q + 500_000;
        base.magic_crit_q = 19 * Q + 999_999;
        let sheet =
            derive_effective_stats(&base, [], &blank(), &rules, LIMITS, CONDITIONS).unwrap();
        assert_eq!(sheet.physical.max_hp(), 100);
        assert_eq!(sheet.cast_speed, 1999);
        assert_eq!(sheet.physical.attack_speed().raw(), 1500 * Q);
        assert_eq!(sheet.magic_crit, 190);
        let funcs = [function(
            "rEvas",
            FunctionOp::Evasion {
                dex: 20,
                level: 1,
                player: true,
            },
        )];
        assert_eq!(
            derive_effective_stats(&base, funcs, &blank(), &rules, LIMITS, CONDITIONS)
                .unwrap()
                .physical
                .evasion()
                .raw(),
            27 * Q
        );
    }
    #[test]
    fn trait_fold_preserves_immunity_provider_counts_and_source_rounding() {
        let handler = |name: &str, value: &str| SkillHandler {
            section: "effects".into(),
            name: name.into(),
            params: BTreeMap::from([(
                "SHOCK".into(),
                super::super::skill::RankValue::Scalar(value.into()),
            )]),
            stat_children: vec![],
            conditions: vec![],
            raw_conditions: vec![],
            section_conditions: vec![],
            section_raw_conditions: vec![],
        };
        let mut traits = BTreeMap::new();
        fold_traits(&mut traits, &handler("DefenceTrait", "0"), 1, 1).unwrap();
        assert!(traits.is_empty());
        fold_traits(&mut traits, &handler("DefenceTrait", "50"), 1, 1).unwrap();
        fold_traits(&mut traits, &handler("DefenceTrait", "100"), 1, 1).unwrap();
        fold_traits(&mut traits, &handler("AttackTrait", "0"), 1, 1).unwrap();
        let state = &traits["SHOCK"];
        assert_eq!(state.defence, ExactRatio::new(3, 2).unwrap());
        assert_eq!(state.defence_providers, 1);
        assert_eq!(state.immunity_providers, 1);
        assert_eq!(state.attack_providers, 1);
    }
    #[test]
    #[allow(clippy::too_many_lines)] // One manifest walk checks all six independent formula families.
    fn every_new_template_oracle_vector_runs_from_loaded_profiles() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle-class-phase3.json"
        ))
        .unwrap();
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let mut count = 0;
        for (family, vectors) in fixture["formulas"].as_object().unwrap() {
            for v in vectors.as_array().unwrap() {
                let input = &v["inputs"];
                let player = input["is_player"].as_bool().unwrap();
                let level = u32::try_from(input["level"].as_u64().unwrap()).unwrap();
                let lm = super::super::level_mod(stats.constants(), level).unwrap();
                let profile = player.then(|| {
                    &rules.profiles().classes[&crate::domain::class::ClassId(
                        u32::try_from(input["class_id"].as_u64().unwrap()).unwrap(),
                    )]
                });
                if family.starts_with("Func") {
                    let profile = profile.unwrap();
                    let mut base = bases();
                    base.level = level;
                    base.m_atk_q = i64::from(profile.magical.base_m_atk) * Q;
                    base.m_def_q = i64::from(profile.magical.base_m_def) * Q;
                    base.cast_speed_q = i64::from(profile.magical.base_m_atk_speed) * Q;
                    let funcs = standard_functions(
                        &profile.base_stats.values(),
                        stats.bonus(),
                        lm,
                        level,
                        true,
                        false,
                    )
                    .unwrap();
                    let sheet =
                        derive_effective_stats(&base, funcs, &blank(), &rules, LIMITS, CONDITIONS)
                            .unwrap();
                    let (q, whole) = match family.as_str() {
                        "FuncMAtkMod" => (sheet.m_atk_q, sheet.m_atk_q / Q),
                        "FuncMDefMod" => (sheet.m_def_q, sheet.m_def_q / Q),
                        "FuncMAtkSpeed" => (
                            sheet
                                .calculator
                                .fold("mAtkSpd", base.cast_speed_q, CONDITIONS)
                                .unwrap(),
                            i64::from(sheet.cast_speed),
                        ),
                        _ => unreachable!(),
                    };
                    assert_eq!(q, v["q_result"].as_i64().unwrap(), "{v}");
                    assert_eq!(whole, v["rounded_result"].as_i64().unwrap(), "{v}");
                } else {
                    let (hp, mp, cp, con, men) = if let Some(profile) = profile {
                        let row = profile.regen_at(level).unwrap();
                        let bonus = |k: StatKind| {
                            ExactRatio::new(
                                i128::from(
                                    stats
                                        .bonus()
                                        .bonus(k, k.of(&profile.base_stats.values()))
                                        .unwrap()
                                        .raw(),
                                ),
                                i128::from(Q),
                            )
                            .unwrap()
                        };
                        (
                            ExactRatio::parse_literal(&row.hp_regen).unwrap(),
                            ExactRatio::parse_literal(&row.mp_regen).unwrap(),
                            ExactRatio::parse_literal(&row.cp_regen).unwrap(),
                            bonus(StatKind::Con),
                            bonus(StatKind::Men),
                        )
                    } else {
                        let npc = &rules.profiles().npcs["keltir"].template.template;
                        (
                            ExactRatio::parse_literal(&npc.hp_regen).unwrap(),
                            ExactRatio::parse_literal(&npc.mp_regen).unwrap(),
                            ExactRatio::whole(0),
                            ExactRatio::whole(1),
                            ExactRatio::whole(1),
                        )
                    };
                    let movement = if player {
                        input["posture"].as_str().unwrap()
                    } else {
                        "npc"
                    };
                    let posture = if movement == "running" {
                        RegenPosture::Moving
                    } else {
                        RegenPosture::Standing
                    };
                    let amounts = regen_amounts(
                        RegenInputs {
                            player,
                            base_hp: hp,
                            base_mp: mp,
                            base_cp: cp,
                            con_bonus: con,
                            men_bonus: men,
                            level_mod: ExactRatio::new(i128::from(lm.raw()), i128::from(Q))
                                .unwrap(),
                            posture,
                        },
                        &StatCalculator::default(),
                        CONDITIONS,
                        rules.regen(),
                    )
                    .unwrap();
                    let amount = if movement == "walking" {
                        // Source-only walking factor1; the zone's movement mode is running.
                        let (base, bonus) = match family.as_str() {
                            "calcHpRegen" => (hp, con),
                            "calcMpRegen" => (mp, men),
                            "calcCpRegen" => (cp, con),
                            _ => unreachable!(),
                        };
                        skill_math::regen_initial(
                            base,
                            ExactRatio::new(i128::from(lm.raw()), i128::from(Q)).unwrap(),
                            bonus,
                        )
                        .unwrap()
                        .to_q()
                        .unwrap()
                    } else {
                        match family.as_str() {
                            "calcHpRegen" => amounts.hp_q,
                            "calcMpRegen" => amounts.mp_q,
                            "calcCpRegen" => amounts.cp_q.unwrap(),
                            _ => unreachable!(),
                        }
                    };
                    assert_eq!(amount, v["q_result"].as_i64().unwrap(), "{v}");
                    let mut current = 0;
                    let mut remainder = 0;
                    for pulse in v["pulse_trace"].as_array().unwrap() {
                        let result =
                            skill_math::restore_resource(current, 1_000_000, remainder, amount)
                                .unwrap();
                        assert_eq!(
                            result.current,
                            u32::try_from(pulse["current"].as_u64().unwrap()).unwrap(),
                            "{v}"
                        );
                        assert_eq!(
                            result.remainder_q,
                            u32::try_from(pulse["remainder_q"].as_u64().unwrap()).unwrap(),
                            "{v}"
                        );
                        assert_eq!(
                            result.granted,
                            u32::try_from(pulse["whole_granted"].as_u64().unwrap()).unwrap(),
                            "{v}"
                        );
                        current = result.current;
                        remainder = result.remainder_q;
                    }
                }
                count += 1;
            }
        }
        assert_eq!(fixture["formulas"].as_object().unwrap().len(), 6);
        assert_eq!(count, 290);
        assert!(rules.skills().get(SkillId(5076)).is_some());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod equipped_tests {
    use super::super::phase3_entity::tests::blank;
    use super::*;
    use crate::domain::class::ClassId;
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        rules_data::{load_rules, RulesSource},
        skill_data::{load_phase3_rules, Phase3DataSource},
    };

    #[test]
    fn every_equipped_oracle_runs_through_complete_player_assembly_and_ordered_sets() {
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle-starter-weapon-phase3.json"
        ))
        .unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let vectors = oracle["formulas"]["FuncMAtkMod"].as_array().unwrap();
        assert_eq!(vectors.len(), 24);
        for v in vectors {
            let class = ClassId(u32::try_from(v["inputs"]["class_id"].as_u64().unwrap()).unwrap());
            let level = u32::try_from(v["inputs"]["level"].as_u64().unwrap()).unwrap();
            let input =
                PlayerStatInputs::from_class(&stats, &classes, &rules, class, level).unwrap();
            let entity = blank();
            let sheet = input.derive(&entity, &rules, false).unwrap();
            assert_eq!(input.bases.m_atk_q, 6 * Q);
            assert_eq!(sheet.m_atk_q, v["q_result"].as_i64().unwrap(), "{v}");
            assert_eq!(sheet.m_atk_q / Q, v["rounded_result"].as_i64().unwrap(), "{v}");
            assert_eq!(input.limits.magic_crit, 200);
            assert_eq!(input.limits.run_speed_q, 250 * Q);
            assert!(sheet.movement_speed().is_ok());
            let mut unarmed = input.clone();
            unarmed
                .functions
                .retain(|f| !(f.key.stat == "mAtk" && f.key.priority == 0));
            let unarmed_sheet = unarmed.derive(&entity, &rules, false).unwrap();
            let expected =
                ExactRatio::parse_literal(v["intermediate"]["unarmed_exact"].as_str().unwrap())
                    .unwrap()
                    .to_q()
                    .unwrap();
            assert_eq!(unarmed_sheet.m_atk_q, expected);
            assert!(unarmed_sheet.m_atk_q > sheet.m_atk_q);
            let mut additive = input.clone();
            additive
                .functions
                .iter_mut()
                .filter(|f| f.key.stat == "mAtk" && f.key.priority == 0)
                .for_each(|f| f.operation = super::super::stat_functions::FunctionOp::Add(5 * Q));
            let additive_sheet = additive.derive(&entity, &rules, false).unwrap();
            assert_eq!(
                additive_sheet.m_atk_q,
                ExactRatio::parse_literal(
                    v["intermediate"]["incorrect_additive_exact"]
                        .as_str()
                        .unwrap()
                )
                .unwrap()
                .to_q()
                .unwrap()
            );
        }
    }
}
