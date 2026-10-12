//! Staged V6 dirty rebuild: load getters precede penalty providers and the final fold.
use super::effective_stats::{EffectiveStats, EquipmentStatContext};
use super::item_sources::{reconcile_sources, refresh_penalties, source_skills};
use super::phase4_checkpoint::EquipmentSourceKey;
use super::skill::SkillId;
use super::skill_math::ExactRatio;
use super::stat_functions::FunctionConditions;
use super::{
    CombatRole, Entity, Phase3Entity, Phase3Rules, Phase4Player, StatError, StatKind, StatRules, Q,
};

/// Complete private reconstruction; publish state, cursor and cache together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshedEquipment {
    /// New retained scalars and penalty installation history.
    pub player: Phase4Player,
    /// Shared Phase3/equipment cursor after changed penalty providers.
    pub next_sequence: u64,
    /// Final getters; callers clamp resources without healing.
    pub effective: EffectiveStats,
}

/// Refresh after equipment, learned skills, attributes, inventory or ordinary effects change.
#[allow(clippy::too_many_arguments)] // Immutable fold inputs plus the shared allocation cursor.
pub fn refresh_player(
    stats: &StatRules,
    classes: &crate::domain::class::ClassRegistry,
    skills: &Phase3Rules,
    entity: &Entity,
    phase3: &Phase3Entity,
    equipment: EquipmentStatContext<'_>,
    next_sequence: u64,
) -> Result<RefreshedEquipment, StatError> {
    let before =
        super::item_modifiers::derive_player(stats, classes, skills, entity, phase3, equipment)?;
    let Some(CombatRole::Player {
        progression: Some(progression),
        ..
    }) = entity.combat.as_ref().map(|c| &c.role)
    else {
        return Err(StatError::Overflow);
    };
    let class = classes
        .get(progression.class_state.current_class_id)
        .ok_or(StatError::Overflow)?;
    let conditions = FunctionConditions {
        weapon_kind: equipment.weapon_kind()?,
        can_resurrect: true,
    };
    let mut player = equipment.player.clone();
    (player.max_load, player.weight_penalty_bonus) = load_getters(
        stats,
        class.base_stats.con,
        &equipment.rules.parts().capacities,
        &before.calculator,
        conditions,
    )?;
    (player.ordinary_capacity, player.quest_capacity) = super::item_inventory::computed_capacities(
        &before.calculator,
        equipment.rules,
        class.race == crate::domain::Race::Dwarf,
        conditions,
    )?;
    let expertise_definition = skills
        .skills()
        .get(SkillId(239))
        .ok_or(StatError::Overflow)?;
    let mut expertise = progression
        .class_state
        .learned_skills
        .iter()
        .filter(|skill| skill.key == expertise_definition.key)
        .map(|skill| skill.level)
        .max()
        .unwrap_or(0);
    for source in player.equipment_sources.values() {
        for skill in
            source_skills(&player, equipment.rules, source).map_err(|_| StatError::Overflow)?
        {
            if skill.id == SkillId(239) {
                expertise = expertise.max(skill.rank);
            }
        }
    }
    if expertise > 7 {
        return Err(StatError::Overflow);
    }
    player.expertise = u8::try_from(expertise).map_err(|_| StatError::Overflow)?;
    // ExpertisePenalty effect is not admitted by the V6 skill registry; its scalar is zero.
    refresh_penalties(&mut player, equipment.rules, 0).map_err(|_| StatError::Overflow)?;
    let mut next = next_sequence;
    for (source, old, new) in [
        (
            EquipmentSourceKey::Overload,
            equipment.player.overload_level,
            player.overload_level,
        ),
        (
            EquipmentSourceKey::GradePenalty { weapon: true },
            equipment.player.weapon_penalty,
            player.weapon_penalty,
        ),
        (
            EquipmentSourceKey::GradePenalty { weapon: false },
            equipment.player.armor_penalty,
            player.armor_penalty,
        ),
    ] {
        if old != new {
            player.equipment_sources.retain(|_, key| *key != source);
            if new > 0 {
                if next == 0
                    || player.equipment_sources.contains_key(&next)
                    || phase3.modifier_install_order.contains_key(&next)
                {
                    return Err(StatError::Overflow);
                }
                let sequence = next;
                next = next.checked_add(1).ok_or(StatError::Overflow)?;
                player.equipment_sources.insert(sequence, source);
            }
        }
    }
    reconcile_sources(&mut player, equipment.rules, &mut next).map_err(|_| StatError::Overflow)?;
    let effective = super::item_modifiers::derive_player(
        stats,
        classes,
        skills,
        entity,
        phase3,
        EquipmentStatContext {
            player: &player,
            rules: equipment.rules,
        },
    )?;
    Ok(RefreshedEquipment {
        player,
        next_sequence: next,
        effective,
    })
}

fn load_getters(
    stats: &StatRules,
    base_con: u32,
    policy: &super::item_rules::ItemCapacityRules,
    calculator: &super::stat_functions::StatCalculator,
    conditions: FunctionConditions<'_>,
) -> Result<(i64, i64), StatError> {
    let con = u32::try_from(
        calculator.fold("CON", super::Scaled::from_int(i64::from(base_con))?.raw(), conditions)?
            / Q,
    )
    .map_err(|_| StatError::Overflow)?;
    let base =
        ExactRatio::new(i128::from(stats.bonus().bonus(StatKind::Con, con)?.raw()), i128::from(Q))?
            .checked_mul(ExactRatio::whole(i64::from(policy.load_multiplier)))?
            .checked_mul(policy.weight_limit.value())?
            .floor()?;
    let base_q = base
        .checked_mul(i128::from(Q))
        .and_then(|v| i64::try_from(v).ok())
        .ok_or(StatError::Overflow)?;
    let getter = |name, base| {
        i32::try_from(calculator.fold(name, base, conditions)? / Q)
            .map(i64::from)
            .map_err(|_| StatError::Overflow)
    };
    let max_load = getter("weightLimit", base_q)?;
    let bonus = getter(
        "weightPenalty",
        policy
            .weight_penalty_base
            .checked_mul(Q)
            .ok_or(StatError::Overflow)?,
    )?;
    Ok((max_load, bonus))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::zone::{
        item_functions::tests::{equip, player},
        item_modifiers::tests::entity,
        phase3_entity::tests::blank,
        stat_functions::{FunctionKey, FunctionOp, StatCalculator, StatFunction},
        EquipSlot, ItemInstance, ItemInstanceId, ItemLocation, Tick,
    };
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        item_data::embedded_phase4_bundle,
        rules_data::{load_rules, RulesSource},
    };

    #[test]
    fn penalty_cannot_feed_back_into_load_selection() {
        let bundle = embedded_phase4_bundle().unwrap();
        let mut definitions = bundle.skills.skills().skills().clone();
        definitions.get_mut(&SkillId(4270)).unwrap().effects[0].stat_children[0].stat =
            "CON".into();
        let skills = (*bundle.skills)
            .clone()
            .with_phase4_skills(
                crate::domain::zone::skill::SkillRegistry::try_new_phase4(definitions).unwrap(),
            )
            .unwrap();
        let mut parts = bundle.items.parts().clone();
        parts.phase3_hash = skills.hash();
        let items = crate::domain::zone::Phase4Rules::try_new(parts).unwrap();
        assert_eq!(items.validate_phase3(&skills).unwrap_err(), "cyclic equipment penalty 4270");
    }

    #[test]
    fn load_getters_match_independent_con_oracle_with_ordered_modifiers() {
        let bundle = embedded_phase4_bundle().unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/item-oracle.json"
        ))
        .unwrap();
        let conditions = FunctionConditions {
            weapon_kind: "FIST",
            can_resurrect: true,
        };
        let function = |priority, operation| StatFunction {
            key: FunctionKey {
                stat: "weightLimit".into(),
                priority,
                install_sequence: 1,
                handler_ordinal: 0,
                function_ordinal: 0,
            },
            operation,
            conditions: Vec::new(),
        };
        let plain = StatCalculator::try_new(Vec::new()).unwrap();
        let modified = StatCalculator::try_new(vec![
            function(1, FunctionOp::Mul(3 * Q / 2)),
            function(2, FunctionOp::Add(100 * Q)),
        ])
        .unwrap();
        let mut count = 0;
        for row in oracle["vectors"].as_array().unwrap().iter().filter(|row| {
            row["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("load-CON-"))
        }) {
            let con = serde_json::from_value(row["inputs"]["con"].clone()).unwrap();
            for (calculator, key) in [(&plain, "max_load"), (&modified, "modified")] {
                let (load, bonus) = load_getters(
                    &stats,
                    con,
                    &bundle.items.parts().capacities,
                    calculator,
                    conditions,
                )
                .unwrap();
                assert_eq!(load, row["expected"][key].as_i64().unwrap(), "{} {key}", row["id"]);
                assert_eq!(bonus, 1);
            }
            count += 1;
        }
        assert_eq!(count, 100);
    }

    #[test]
    fn dirty_refresh_installs_penalties_once_and_rolls_back_cursor_exhaustion() {
        let bundle = embedded_phase4_bundle().unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let entity = entity(&stats);
        let phase3 = blank();
        let mut owned = player();
        equip(&mut owned, 71, 0, EquipSlot::Rhand, 1);
        equip(&mut owned, 58, 0, EquipSlot::Chest, 2);
        let refresh = |player: &Phase4Player, next| {
            refresh_player(
                &stats,
                &classes,
                &bundle.skills,
                &entity,
                &phase3,
                EquipmentStatContext {
                    player,
                    rules: &bundle.items,
                },
                next,
            )
        };
        let original = owned.clone();
        assert!(refresh(&owned, u64::MAX).is_err());
        assert_eq!(owned, original);
        let first = refresh(&owned, 100).unwrap();
        assert_eq!((first.player.weapon_penalty, first.player.armor_penalty), (2, 1));
        assert_eq!(first.player.expertise, 0);
        let weight = bundle.items.parts().items[&17].weight;
        let count = u64::try_from(
            (first.player.max_load * 7 / 10 - first.player.weight) / i64::from(weight),
        )
        .unwrap();
        let mut weighted = first.player;
        let id = ItemInstanceId::new(99).unwrap();
        weighted.inventory.insert(
            id,
            ItemInstance {
                id,
                template_id: 17,
                count,
                location: ItemLocation::Inventory,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: 1,
            },
        );
        let loaded = refresh(&weighted, first.next_sequence).unwrap();
        assert_eq!(loaded.player.overload_level, 2);
        assert!(!loaded.player.overloaded);
        let repeat = refresh(&loaded.player, loaded.next_sequence).unwrap();
        assert_eq!(repeat.player, loaded.player);
        assert_eq!(repeat.next_sequence, loaded.next_sequence);
        assert_eq!(repeat.effective, loaded.effective);
        assert_eq!(entity.combat.as_ref().unwrap().hp, 10);
    }
}
