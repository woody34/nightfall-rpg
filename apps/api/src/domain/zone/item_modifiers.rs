//! V6 System modifiers share calculator ordering with learned passives and ordinary effects.
//! Equipment providers are reconstructed independently of stayAfterDeath (D16).
use std::collections::{BTreeMap, BTreeSet};

use super::effective_stats::{
    compile_handler, finish_effective_stats, fold_traits, CompiledModifiers, EffectiveStats,
    EquipmentStatContext, PlayerStatInputs,
};
use super::effects::{continuous_handler, ActiveHandler, StatusFlags};
use super::item_sources::{desired_sources, source_option, source_skills, SET_ATTRIBUTES_ORDINAL};
use super::phase4_checkpoint::EquipmentSourceKey;
use super::skill::{SkillCondition, SkillId, SkillRef};
use super::stat_functions::{
    FunctionConditions, FunctionKey, FunctionOp, StatCalculator, StatFunction,
};
use super::{
    CombatRole, Entity, EquipSlot, Phase3Entity, Phase3Rules, RestoredModifierSource, Scaled,
    StatError, StatRules, Q,
};

struct Provider<'a> {
    reference: SkillRef,
    sequence: u64,
    ordinal: u32,
    selected: Option<&'a [ActiveHandler]>,
}

/// Complete equipment-aware player reconstruction used by the V6 runtime cache.
pub fn derive_player(
    stats: &StatRules,
    classes: &crate::domain::class::ClassRegistry,
    skills: &Phase3Rules,
    entity: &Entity,
    phase3: &Phase3Entity,
    equipment: EquipmentStatContext<'_>,
) -> Result<EffectiveStats, StatError> {
    let combat = entity.combat.as_ref().ok_or(StatError::Overflow)?;
    let CombatRole::Player {
        progression: Some(progression),
        ..
    } = &combat.role
    else {
        return Err(StatError::Overflow);
    };
    let class_id = progression.class_state.current_class_id;
    let class = classes.get(class_id).ok_or(StatError::Overflow)?;
    let level = combat.sheet.level();
    let mut inputs =
        PlayerStatInputs::from_equipment(stats, classes, skills, class_id, level, equipment)?;
    let mut compiled = compile(phase3, skills, equipment, entity, level)?;
    compiled.functions.extend(
        inputs
            .functions
            .into_iter()
            .filter(|f| f.key.install_sequence != 0),
    );
    let conditions = FunctionConditions {
        weapon_kind: equipment.weapon_kind()?,
        can_resurrect: true,
    };
    let calculator = StatCalculator::try_new(compiled.functions.clone())?;
    let mut attributes = class.base_stats;
    for (name, value) in [
        ("STR", &mut attributes.str),
        ("DEX", &mut attributes.dex),
        ("CON", &mut attributes.con),
        ("INT", &mut attributes.int),
        ("MEN", &mut attributes.men),
        ("WIT", &mut attributes.wit),
    ] {
        *value = u32::try_from(
            calculator
                .fold(name, Scaled::from_int(i64::from(*value))?.raw(), conditions)?
                .checked_div(Q)
                .ok_or(StatError::Overflow)?,
        )
        .map_err(|_| StatError::Overflow)?;
    }
    inputs.functions = super::stat_functions::standard_functions(
        &attributes,
        stats.bonus(),
        super::level_mod(stats.constants(), level)?,
        level,
        true,
        equipment.player.paperdoll.contains_key(&EquipSlot::Rhand),
    )?;
    let slots = equipment
        .rules
        .parts()
        .class_slots
        .iter()
        .find(|row| row.base_class_id == class.base_class_id.0)
        .ok_or(StatError::Overflow)?;
    super::item_functions::subtract_defence_slots(
        &mut inputs.functions,
        equipment.player,
        equipment.rules,
        slots,
    )?;
    compiled.functions.extend(inputs.functions);
    finish_effective_stats(&inputs.bases, compiled, skills, inputs.limits, conditions)
}

fn compile<'a>(
    phase3: &'a Phase3Entity,
    skills: &Phase3Rules,
    equipment: EquipmentStatContext<'_>,
    entity: &Entity,
    level: u32,
) -> Result<CompiledModifiers, StatError> {
    let expected =
        desired_sources(equipment.player, equipment.rules).map_err(|_| StatError::Overflow)?;
    let actual: BTreeSet<_> = equipment
        .player
        .equipment_sources
        .values()
        .cloned()
        .collect();
    if expected != actual || actual.len() != equipment.player.equipment_sources.len() {
        return Err(StatError::Overflow);
    }
    let mut winners: BTreeMap<SkillId, Provider<'a>> = BTreeMap::new();
    let mut installed = Vec::new();
    for (sequence, source) in &phase3.modifier_install_order {
        let (reference, selected) = match source {
            RestoredModifierSource::Passive(reference) => (*reference, None),
            RestoredModifierSource::Effect(id) => {
                let effect = phase3
                    .effects
                    .entries()
                    .get(id)
                    .ok_or(StatError::Overflow)?;
                (effect.skill, Some(effect.handlers.as_slice()))
            },
            RestoredModifierSource::DeathPenalty => (
                SkillRef {
                    id: SkillId(5076),
                    rank: u32::from(
                        phase3
                            .player
                            .as_ref()
                            .ok_or(StatError::Overflow)?
                            .death_penalty_level,
                    ),
                },
                None,
            ),
        };
        let provider = Provider {
            reference,
            sequence: *sequence,
            ordinal: 0,
            selected,
        };
        if matches!(source, RestoredModifierSource::Passive(_)) {
            winners.insert(reference.id, provider);
        } else {
            installed.push(provider);
        }
    }
    let mut result = CompiledModifiers {
        functions: Vec::new(),
        traits: BTreeMap::new(),
        flags: StatusFlags::default(),
    };
    let by_key: BTreeMap<_, _> = equipment
        .player
        .equipment_sources
        .iter()
        .map(|(sequence, key)| (key, *sequence))
        .collect();
    for (key, sequence) in by_key {
        if sequence == 0 || phase3.modifier_install_order.contains_key(&sequence) {
            return Err(StatError::Overflow);
        }
        extra_functions(&mut result.functions, equipment, key, sequence)?;
        let mut next_ordinal = 1_u32;
        for reference in source_skills(equipment.player, equipment.rules, key)
            .map_err(|_| StatError::Overflow)?
        {
            let definition = skills
                .skills()
                .get(reference.id)
                .ok_or(StatError::Overflow)?;
            if reference.rank == 0
                || reference.rank > definition.max_level
                || definition.runtime_status != crate::domain::class::RuntimeSkillStatus::Executable
            {
                return Err(StatError::Overflow);
            }
            let ordinal = next_ordinal;
            next_ordinal = next_ordinal
                .checked_add(
                    u32::try_from(definition.effects.len()).map_err(|_| StatError::Overflow)?,
                )
                .ok_or(StatError::Overflow)?;
            if definition.operate_type != "P" {
                continue;
            }
            // Equal-rank learned providers win; otherwise source-key order is the tie-break.
            if winners
                .get(&reference.id)
                .is_none_or(|old| reference.rank > old.reference.rank)
            {
                winners.insert(
                    reference.id,
                    Provider {
                        reference,
                        sequence,
                        ordinal,
                        selected: None,
                    },
                );
            }
        }
    }
    installed.extend(winners.into_values());
    installed.sort_by_key(|provider| (provider.sequence, provider.ordinal));
    for provider in installed {
        compile_provider(&mut result, &provider, skills, equipment, entity, level)?;
    }
    Ok(result)
}

fn compile_provider(
    result: &mut CompiledModifiers,
    provider: &Provider<'_>,
    skills: &Phase3Rules,
    equipment: EquipmentStatContext<'_>,
    entity: &Entity,
    level: u32,
) -> Result<(), StatError> {
    let reference = provider.reference;
    let skill = skills
        .skills()
        .get(reference.id)
        .ok_or(StatError::Overflow)?;
    if provider.sequence == 0
        || reference.rank == 0
        || reference.rank > skill.max_level
        || skill.runtime_status != crate::domain::class::RuntimeSkillStatus::Executable
    {
        return Err(StatError::Overflow);
    }
    let matches = |condition: &SkillCondition| {
        condition_matches(condition, equipment, entity, level, reference.rank)
    };
    for (index, handler) in skill.effects.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| StatError::Overflow)?;
        if !continuous_handler(&handler.name) {
            continue;
        }
        if let Some(selected) = provider.selected {
            if !selected.iter().any(|h| h.xml_ordinal == index) {
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
                eligible &= matches(condition)?;
            }
            if !eligible {
                continue;
            }
        }
        // Direct option functions own handler0; skill handler ranges follow source order.
        let ordinal = provider
            .ordinal
            .checked_add(index)
            .ok_or(StatError::Overflow)?;
        let mut functions = Vec::new();
        compile_handler(
            &mut functions,
            handler,
            skill,
            reference.rank,
            provider.sequence,
            ordinal,
        )?;
        for mut function in functions {
            let mut eligible = true;
            for condition in &function.conditions {
                eligible &= matches(condition)?;
            }
            if eligible {
                function.conditions.clear();
                result.functions.push(function);
            }
        }
        fold_traits(&mut result.traits, handler, reference.rank, skill.max_level)?;
        result.flags = result.flags.union(match handler.name.as_str() {
            "Stun" => StatusFlags::STUN,
            "Sleep" => StatusFlags::SLEEP,
            "Root" => StatusFlags::ROOT,
            _ => StatusFlags::default(),
        });
    }
    Ok(())
}

fn extra_functions(
    out: &mut Vec<StatFunction>,
    equipment: EquipmentStatContext<'_>,
    source: &EquipmentSourceKey,
    sequence: u64,
) -> Result<(), StatError> {
    if let Some(option) =
        source_option(equipment.player, equipment.rules, source).map_err(|_| StatError::Overflow)?
    {
        for f in &option.functions {
            if !f.conditions.is_empty() {
                return Err(StatError::Overflow);
            }
            let value = f.value.value().to_q()?;
            let operation = match f.operation.as_str() {
                "SET" => FunctionOp::Set(value),
                "ADD" => FunctionOp::Add(value),
                "SUB" => FunctionOp::Sub(value),
                "MUL" => FunctionOp::Mul(value),
                "DIV" if value > 0 => FunctionOp::Div(value),
                _ => return Err(StatError::Overflow),
            };
            out.push(StatFunction {
                key: FunctionKey {
                    stat: f.stat.clone(),
                    priority: f.order,
                    install_sequence: sequence,
                    handler_ordinal: 0,
                    function_ordinal: f.xml_ordinal,
                },
                operation,
                conditions: Vec::new(),
            });
        }
    }
    if let EquipmentSourceKey::ArmorSet {
        set,
        ordinal: SET_ATTRIBUTES_ORDINAL,
        ..
    } = source
    {
        let set = equipment
            .rules
            .parts()
            .armor_sets
            .get(set)
            .ok_or(StatError::Overflow)?;
        for (ordinal, (stat, amount)) in set.base_stats.iter().enumerate() {
            out.push(StatFunction {
                key: FunctionKey {
                    stat: stat.clone(),
                    priority: 1,
                    install_sequence: sequence,
                    handler_ordinal: 0,
                    function_ordinal: u32::try_from(ordinal).map_err(|_| StatError::Overflow)?,
                },
                operation: FunctionOp::Add(Scaled::from_int(i64::from(*amount))?.raw()),
                conditions: Vec::new(),
            });
        }
    }
    Ok(())
}

fn condition_matches(
    condition: &SkillCondition,
    equipment: EquipmentStatContext<'_>,
    entity: &Entity,
    level: u32,
    rank: u32,
) -> Result<bool, StatError> {
    let at = |slot| {
        equipment
            .player
            .paperdoll
            .get(&slot)
            .and_then(|id| equipment.player.inventory.get(id))
            .and_then(|item| equipment.rules.parts().items.get(&item.template_id))
    };
    match condition {
        SkillCondition::UsingKind { weapon_kinds } => {
            if weapon_kinds
                .iter()
                .any(|kind| matches!(kind.as_str(), "LIGHT" | "MAGIC" | "HEAVY"))
            {
                let Some(chest) = at(EquipSlot::Chest) else {
                    return Ok(false);
                };
                Ok(weapon_kinds.contains(&chest.armor_type)
                    && (chest.bodypart == "onepiece"
                        || at(EquipSlot::Legs)
                            .is_some_and(|legs| weapon_kinds.contains(&legs.armor_type))))
            } else {
                Ok(equipment
                    .player
                    .paperdoll
                    .keys()
                    .filter_map(|slot| at(*slot))
                    .any(|def| {
                        weapon_kinds.contains(&def.weapon_type)
                            || weapon_kinds.contains(&def.armor_type)
                    }))
            }
        },
        SkillCondition::UsingSlot { slot } => {
            Ok(at(EquipSlot::Rhand).is_some_and(|weapon| weapon.bodypart == *slot))
        },
        SkillCondition::PlayerLevelRange { ranges } => {
            let range = rank
                .checked_sub(1)
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|n| ranges.get(n))
                .ok_or(StatError::Overflow)?;
            Ok(range.minimum <= level && level <= range.maximum)
        },
        // V6 currently has running movement, no sitting/walking/flying action state.
        // ConditionPlayerState.java:48–66: standing excludes movement, not merely sitting.
        SkillCondition::PlayerResting { value } | SkillCondition::PlayerFlyMounted { value } => {
            Ok(!value)
        },
        SkillCondition::PlayerRunning { value } | SkillCondition::PlayerCanResurrect { value } => {
            Ok(*value)
        },
        SkillCondition::PlayerMoving { value } => Ok(*value == entity.dest.is_some()),
        SkillCondition::PlayerStanding { value } => Ok(*value == entity.dest.is_none()),
        SkillCondition::And { conditions, .. } => {
            for child in conditions {
                if !condition_matches(child, equipment, entity, level, rank)? {
                    return Ok(false);
                }
            }
            Ok(true)
        },
        SkillCondition::Or { conditions, .. } => {
            for child in conditions {
                if condition_matches(child, equipment, entity, level, rank)? {
                    return Ok(true);
                }
            }
            Ok(false)
        },
        SkillCondition::Not { conditions, .. } if conditions.len() == 1 => condition_matches(
            conditions.first().ok_or(StatError::Overflow)?,
            equipment,
            entity,
            level,
            rank,
        )
        .map(|v| !v),
        SkillCondition::PlayerInsideZoneIds { .. }
        | SkillCondition::PlayerCanEscape { .. }
        | SkillCondition::PlayerWeight { .. }
        | SkillCondition::PlayerInventorySize { .. }
        | SkillCondition::Not { .. }
        | SkillCondition::Unsupported { .. } => Err(StatError::Overflow),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
pub(crate) mod tests {
    use super::super::item_functions::tests::{equip, player};
    use super::super::item_sources::{reconcile_sets, reconcile_sources};
    use super::super::phase3_entity::tests::blank;
    use super::super::{
        CombatState, EntityId, EntityKind, Fixed, PlayerProgression, SessionGeneration, Speed,
        StatSheet, TargetingState, Tick, Vec2Fixed,
    };
    use super::*;
    use crate::domain::character_progression::{
        CharacterAppearance, CharacterIdentity, ClassState,
    };
    use crate::domain::class::ClassId;
    use crate::domain::{AccountId, Race};
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        item_data::embedded_phase4_bundle,
        rules_data::{load_rules, RulesSource},
    };

    pub(crate) fn entity(stats: &StatRules) -> Entity {
        Entity {
            id: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
            kind: EntityKind::Player,
            name: "System sources".into(),
            pos: Vec2Fixed::from_tiles(10, 10),
            dest: None,
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            targeting: TargetingState::default(),
            ai: None,
            combat: Some(CombatState {
                role: CombatRole::Player {
                    class: "human_fighter".into(),
                    xp: 0,
                    progression: Some(Box::new(PlayerProgression {
                        identity: CharacterIdentity {
                            account_id: AccountId::from_uuid(uuid::Uuid::from_u128(1)),
                            race: Race::Human,
                            base_class_id: ClassId(0),
                            appearance: CharacterAppearance::default(),
                        },
                        class_state: ClassState::new(ClassId(0)),
                        max_cp: 0,
                    })),
                },
                sheet: StatSheet::for_player(
                    stats,
                    stats.class("human_fighter").unwrap(),
                    84,
                    None,
                )
                .unwrap(),
                hp: 10,
                mp: 10,
                attack_range: Fixed::from_raw(1000),
                collision_radius: Fixed::from_raw(100),
                incarnation: 1,
                auto_attack: false,
                chasing: false,
                swing: None,
                ready_at: Tick(0),
                protected_until: None,
            }),
        }
    }
    #[test]
    fn set_and_pole_skills_match_independent_oracle_without_timed_effects() {
        let bundle = embedded_phase4_bundle().unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let entity = entity(&stats);
        let phase3 = blank();
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/item-oracle.json"
        ))
        .unwrap();
        let mut count = 0;
        for vector in oracle["vectors"].as_array().unwrap().iter().filter(|v| {
            matches!(
                v["id"].as_str().unwrap(),
                "gear-19-HEAVY-set" | "gear-20-LIGHT-set" | "gear-21-MAGIC-set" | "gear-07-POLE"
            )
        }) {
            let mut player = player();
            for gear in vector["inputs"]["equipment"].as_array().unwrap() {
                equip(
                    &mut player,
                    u32::try_from(gear[0].as_u64().unwrap()).unwrap(),
                    u32::try_from(gear[1].as_u64().unwrap()).unwrap(),
                    serde_json::from_value(gear[2].clone()).unwrap(),
                    gear[3].as_u64().unwrap(),
                );
            }
            player.active_sets = reconcile_sets(&player, &bundle.items, None).unwrap();
            let mut next = 100;
            reconcile_sources(&mut player, &bundle.items, &mut next).unwrap();
            let before = player.clone();
            let result = derive_player(
                &stats,
                &classes,
                &bundle.skills,
                &entity,
                &phase3,
                EquipmentStatContext {
                    player: &player,
                    rules: &bundle.items,
                },
            )
            .unwrap();
            for (name, actual) in [
                ("pAtk", result.physical.p_atk().raw()),
                ("pDef", result.physical.p_def().raw()),
                ("mAtk", result.m_atk_q),
                ("mDef", result.m_def_q),
            ] {
                assert_eq!(
                    actual,
                    vector["expected"]["q"][name].as_i64().unwrap(),
                    "{} {name}",
                    vector["id"]
                );
            }
            assert_eq!(
                u64::from(result.physical.max_hp()),
                vector["expected"]["getters"]["maxHp"].as_u64().unwrap()
            );
            assert_eq!(phase3.effects.entries().len(), 0);
            assert_eq!(before, player);
            count += 1;
        }
        assert_eq!(count, 4);
    }
    #[test]
    fn equal_learned_provider_is_applied_once_and_dead_rebuild_keeps_system_sources() {
        let bundle = embedded_phase4_bundle().unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let mut entity = entity(&stats);
        let mut player = player();
        equip(&mut player, 15, 0, EquipSlot::Rhand, 1);
        player.weapon_penalty = 2;
        player.armor_penalty = 1;
        player.overload_level = 2;
        let mut next = 100;
        reconcile_sources(&mut player, &bundle.items, &mut next).unwrap();
        let context = EquipmentStatContext {
            player: &player,
            rules: &bundle.items,
        };
        let mut phase3 = blank();
        let living =
            derive_player(&stats, &classes, &bundle.skills, &entity, &phase3, context).unwrap();
        phase3.modifier_install_order.insert(
            50,
            RestoredModifierSource::Passive(SkillRef {
                id: SkillId(3599),
                rank: 1,
            }),
        );
        let overlap =
            derive_player(&stats, &classes, &bundle.skills, &entity, &phase3, context).unwrap();
        let conditions = FunctionConditions {
            weapon_kind: "POLE",
            can_resurrect: true,
        };
        assert_eq!(
            living
                .calculator
                .fold("atkCountMax", Q, conditions)
                .unwrap(),
            overlap
                .calculator
                .fold("atkCountMax", Q, conditions)
                .unwrap()
        );
        entity.targeting.dead = true;
        entity.combat.as_mut().unwrap().hp = 0;
        let dead =
            derive_player(&stats, &classes, &bundle.skills, &entity, &phase3, context).unwrap();
        assert_eq!(dead, overlap);
        assert_eq!(entity.combat.as_ref().unwrap().hp, 0);
        assert!(phase3.effects.entries().is_empty());
        // This is a cold-fold check only; actor death/checkpoint/resurrection is separate.
        // Skill3599 is supplied by the pole item, the pole provider and learned overlap,
        // but its source ADD4 contributes once (DP03500–03599.xml:1695–1705).
        assert_eq!(
            living
                .calculator
                .fold("atkCountMax", Q, conditions)
                .unwrap(),
            5 * Q
        );
    }
}
