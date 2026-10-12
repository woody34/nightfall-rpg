//! V6 item calculator functions, preserving XML order within the shared install sequence.
//! Pinned FuncEnchant.java:43–155 and EnchantItemHPBonusData.java:116–126.

use super::item_rules::{ClassSlotRule, ItemDefinition};
use super::phase4_checkpoint::EquipmentSourceKey;
use super::stat_functions::{FunctionKey, FunctionOp, StatFunction};
use super::{EquipSlot, ItemInstance, Phase4Player, Phase4Rules, Scaled, StatError};

/// Compile a single installed item's source functions; no provider allocation or hooks.
/// Unsupported conditions reject instead of silently dropping part of an item.
pub fn item_functions(
    item: &ItemInstance,
    sequence: u64,
    rules: &Phase4Rules,
) -> Result<Vec<StatFunction>, StatError> {
    if sequence == 0 || item.slot.is_none() {
        return Err(StatError::Overflow);
    }
    let definition = rules
        .parts()
        .items
        .get(&item.template_id)
        .ok_or(StatError::Overflow)?;
    let mut result = Vec::new();
    for source in &definition.functions {
        if !source.conditions.is_empty() {
            return Err(StatError::Overflow);
        }
        let value = source.value.value().to_q()?;
        let operation = match source.operation.as_str() {
            "SET" => FunctionOp::Set(value),
            "ADD" => FunctionOp::Add(value),
            "SUB" => FunctionOp::Sub(value),
            "MUL" => FunctionOp::Mul(value),
            "DIV" if value > 0 => FunctionOp::Div(value),
            "ENCHANT" => {
                FunctionOp::Add(enchant_bonus(definition, item.enchant, &source.stat, rules)?)
            },
            _ => return Err(StatError::Overflow),
        };
        result.push(function(&source.stat, source.order, sequence, source.xml_ordinal, operation));
    }
    if definition.enchant_hp {
        let ordinal = definition
            .functions
            .iter()
            .map(|f| f.xml_ordinal)
            .max()
            .map_or(Ok(0), |last| last.checked_add(1).ok_or(StatError::Overflow))?;
        result.push(function(
            "maxHp",
            rules.parts().func_enchant.hp_order,
            sequence,
            ordinal,
            FunctionOp::Add(enchant_hp(definition, item.enchant, rules)?),
        ));
    }
    Ok(result)
}

fn function(
    stat: &str,
    priority: i32,
    sequence: u64,
    ordinal: u32,
    operation: FunctionOp,
) -> StatFunction {
    StatFunction {
        key: FunctionKey {
            stat: stat.into(),
            priority,
            install_sequence: sequence,
            handler_ordinal: 0,
            function_ordinal: ordinal,
        },
        operation,
        conditions: Vec::new(),
    }
}

fn enchant_bonus(
    definition: &ItemDefinition,
    enchant: u32,
    stat: &str,
    rules: &Phase4Rules,
) -> Result<i64, StatError> {
    let policy = &rules.parts().func_enchant;
    let safe = u64::from(enchant.min(policy.safe_levels));
    let over = u64::from(enchant.saturating_sub(policy.safe_levels));
    let (unit, multiplier) = if matches!(stat, "pDef" | "mDef") {
        (1, policy.defence_over_multiplier)
    } else if stat == "mAtk" || definition.kind == "Weapon" {
        let row = policy
            .attack
            .iter()
            .find(|row| row.grade == definition.grade)
            .ok_or(StatError::Overflow)?;
        let unit = if stat == "mAtk" {
            row.m_atk
        } else if matches!(definition.weapon_type.as_str(), "BOW" | "CROSSBOW")
            && (definition.bodypart == "lrhand"
                || matches!(definition.grade.as_str(), "NONE" | "D"))
        {
            row.p_atk_bow
        } else if definition.bodypart == "lrhand" {
            row.p_atk_two_hand
        } else {
            row.p_atk
        };
        (unit, policy.attack_over_multiplier)
    } else {
        // FuncEnchant.java:66–85,154: shield sDef is neither pDef/mDef nor a weapon.
        return Ok(0);
    };
    let amount = over
        .checked_mul(u64::from(multiplier))
        .and_then(|v| v.checked_add(safe))
        .and_then(|v| v.checked_mul(u64::from(unit)))
        .ok_or(StatError::Overflow)?;
    Scaled::from_int(i64::try_from(amount).map_err(|_| StatError::Overflow)?).map(Scaled::raw)
}

fn enchant_hp(
    definition: &ItemDefinition,
    enchant: u32,
    rules: &Phase4Rules,
) -> Result<i64, StatError> {
    if enchant == 0 {
        return Ok(0);
    }
    let grade = rules
        .parts()
        .grades
        .iter()
        .find(|row| row.grade == definition.grade)
        .ok_or(StatError::Overflow)?;
    let Some(row) = rules
        .parts()
        .enchant_hp
        .iter()
        .find(|row| row.grade == grade.shot_grade)
    else {
        return Ok(0);
    };
    let index = usize::try_from(enchant)
        .map_err(|_| StatError::Overflow)?
        .min(row.bonuses.len());
    let Some(index) = index.checked_sub(1) else {
        return Ok(0);
    };
    let mut bonus = i64::from(*row.bonuses.get(index).ok_or(StatError::Overflow)?);
    if definition.bodypart == "onepiece" {
        bonus = bonus
            .checked_mul(3)
            .and_then(|v| v.checked_div(2))
            .ok_or(StatError::Overflow)?;
    }
    Scaled::from_int(bonus).map(Scaled::raw)
}

/// Rebuild direct item functions from retained provider identity; unrelated System skills
/// are compiled by the shared skill-provider fold, never converted to timed effects.
pub fn equipped_functions(
    player: &Phase4Player,
    rules: &Phase4Rules,
) -> Result<Vec<StatFunction>, StatError> {
    let mut result = Vec::new();
    for id in player.paperdoll.values() {
        let source = EquipmentSourceKey::Item { instance: *id };
        let mut sequences = player
            .equipment_sources
            .iter()
            .filter(|(_, key)| **key == source);
        let (sequence, _) = sequences.next().ok_or(StatError::Overflow)?;
        if sequences.next().is_some() {
            return Err(StatError::Overflow);
        }
        let item = player.inventory.get(id).ok_or(StatError::Overflow)?;
        result.extend(item_functions(item, *sequence, rules)?);
    }
    Ok(result)
}

/// Occupied bases are removed inside each priority1 builtin before its multiplier.
/// FuncPDefMod.java:44–70 / FuncMDefMod.java:44–66; full armor also occupies legs.
pub fn subtract_defence_slots(
    functions: &mut Vec<StatFunction>,
    player: &Phase4Player,
    rules: &Phase4Rules,
    slots: &ClassSlotRule,
) -> Result<(), StatError> {
    let full_armor = player
        .paperdoll
        .get(&EquipSlot::Chest)
        .and_then(|id| player.inventory.get(id))
        .and_then(|item| rules.parts().items.get(&item.template_id))
        .is_some_and(|item| item.bodypart == "onepiece");
    for (stat, bases) in [("pDef", &slots.p_def_slots), ("mDef", &slots.m_def_slots)] {
        let mut total = 0_i64;
        for (slot, base) in bases {
            if player.paperdoll.contains_key(slot)
                || (stat == "pDef" && *slot == EquipSlot::Legs && full_armor)
            {
                total = total
                    .checked_add(i64::from(*base))
                    .ok_or(StatError::Overflow)?;
            }
        }
        let builtin = functions
            .iter_mut()
            .find(|f| f.key.stat == stat && f.key.priority == 1 && f.key.install_sequence == 0)
            .ok_or(StatError::Overflow)?;
        if builtin.key.function_ordinal != 0 {
            return Err(StatError::Overflow);
        }
        builtin.key.function_ordinal = 1;
        functions.push(function(stat, 1, 0, 0, FunctionOp::Sub(Scaled::from_int(total)?.raw())));
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
pub(super) mod tests {
    use super::super::effective_stats::{EquipmentStatContext, PlayerStatInputs};
    use super::super::phase4_checkpoint::StarterKitReceipt;
    use super::super::stat_functions::{FunctionConditions, StatCalculator};
    use super::super::{ItemInstanceId, ItemLocation, Tick, Q};
    use super::*;
    use crate::domain::class::ClassId;
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        item_data::embedded_phase4_bundle,
        rules_data::{load_rules, RulesSource},
    };
    use std::collections::{BTreeMap, BTreeSet};

    pub(crate) fn player() -> Phase4Player {
        Phase4Player {
            inventory: BTreeMap::new(),
            paperdoll: BTreeMap::new(),
            equipment_sources: BTreeMap::new(),
            auto_shots: BTreeSet::new(),
            item_reuse: Vec::new(),
            active_sets: BTreeMap::new(),
            weight: 0,
            max_load: 1,
            weight_penalty_bonus: 1,
            overload_level: 0,
            overloaded: false,
            expertise: 7,
            weapon_penalty: 0,
            armor_penalty: 0,
            inventory_revision: 1,
            starter_kit_receipt: StarterKitReceipt {
                manifest_version: 1,
                manifest_hash: [1; 32],
                source_class: ClassId(0),
                request_id: uuid::Uuid::from_u128(1),
                materialized_revision: 1,
            },
        }
    }
    pub(crate) fn equip(
        player: &mut Phase4Player,
        template: u32,
        enchant: u32,
        slot: EquipSlot,
        sequence: u64,
    ) {
        let id = ItemInstanceId::new(sequence).unwrap();
        player.inventory.insert(
            id,
            ItemInstance {
                id,
                template_id: template,
                count: 1,
                location: ItemLocation::Paperdoll,
                slot: Some(slot),
                enchant,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: 1,
            },
        );
        player.paperdoll.insert(slot, id);
        player
            .equipment_sources
            .insert(sequence, EquipmentSourceKey::Item { instance: id });
    }
    #[test]
    fn source_oracle_direct_equipment_and_kits_fold_from_raw_class_bases() {
        let bundle = embedded_phase4_bundle().unwrap();
        let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/item-oracle.json"
        ))
        .unwrap();
        let mut checked = 0;
        for vector in oracle["vectors"].as_array().unwrap() {
            let id = vector["id"].as_str().unwrap();
            if !id.starts_with("gear-")
                || (19..=21).any(|n| id.starts_with(&format!("gear-{n}")))
                || id == "gear-07-POLE"
            {
                continue;
            }
            let input = &vector["inputs"];
            let stem =
                std::path::Path::new(input["class_file"].as_str().unwrap_or("HumanFighter.xml"))
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_ascii_lowercase();
            let class = bundle
                .items
                .parts()
                .class_slots
                .iter()
                .find(|row| row.key.replace('_', "") == stem)
                .unwrap();
            let mut player = player();
            for gear in input["equipment"].as_array().unwrap() {
                equip(
                    &mut player,
                    u32::try_from(gear[0].as_u64().unwrap()).unwrap(),
                    u32::try_from(gear[1].as_u64().unwrap()).unwrap(),
                    serde_json::from_value(gear[2].clone()).unwrap(),
                    gear[3].as_u64().unwrap(),
                );
            }
            let context = EquipmentStatContext {
                player: &player,
                rules: &bundle.items,
            };
            let assembled = PlayerStatInputs::from_equipment(
                &stats,
                &classes,
                &bundle.skills,
                ClassId(class.base_class_id),
                u32::try_from(input["level"].as_u64().unwrap()).unwrap(),
                context,
            )
            .unwrap();
            let calculator = StatCalculator::try_new(assembled.functions).unwrap();
            let bases = &assembled.bases;
            let conditions = FunctionConditions {
                weapon_kind: context.weapon_kind().unwrap(),
                can_resurrect: true,
            };
            for (stat, initial) in [
                ("pAtk", bases.p_atk_q),
                ("mAtk", bases.m_atk_q),
                ("pDef", bases.p_def_q),
                ("mDef", bases.m_def_q),
                ("critRate", bases.crit_q),
                ("pAtkSpd", bases.attack_speed_q),
                ("maxHp", bases.max_hp_q),
                ("maxMp", bases.max_mp_q),
                ("maxCp", bases.max_cp_q),
                ("mAtkSpd", bases.cast_speed_q),
                ("runSpd", bases.run_speed_q),
            ] {
                assert_eq!(
                    calculator.fold(stat, initial, conditions).unwrap(),
                    vector["expected"]["q"][stat].as_i64().unwrap(),
                    "{id}: {stat}"
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 20);
    }
    #[test]
    fn set_then_enchant_preserves_source_order_and_shield_enchant_is_noop() {
        let bundle = embedded_phase4_bundle().unwrap();
        let mut player = player();
        equip(&mut player, 83, 4, EquipSlot::Rhand, 5);
        let conditions = FunctionConditions {
            weapon_kind: "SWORD",
            can_resurrect: false,
        };
        let functions = equipped_functions(&player, &bundle.items).unwrap();
        let calc = StatCalculator::try_new(functions.clone()).unwrap();
        let direct = calc.fold("pAtk", 123 * Q, conditions).unwrap();
        let mut reversed = functions;
        let ordinals: Vec<_> = reversed
            .iter()
            .filter(|f| f.key.stat == "pAtk")
            .map(|f| f.key.function_ordinal)
            .collect();
        assert_eq!(ordinals.len(), 2);
        for function in reversed.iter_mut().filter(|f| f.key.stat == "pAtk") {
            function.key.function_ordinal = if function.key.function_ordinal == ordinals[0] {
                ordinals[1]
            } else {
                ordinals[0]
            };
        }
        let backwards = StatCalculator::try_new(reversed)
            .unwrap()
            .fold("pAtk", 123 * Q, conditions)
            .unwrap();
        assert_eq!(direct - backwards, 10 * Q);
        equip(&mut player, 103, 6, EquipSlot::Lhand, 6);
        let shield = bundle.items.parts().items.get(&103).unwrap();
        assert_eq!(enchant_bonus(shield, 6, "sDef", &bundle.items).unwrap(), 0);
        assert!(enchant_hp(shield, 6, &bundle.items).unwrap() > 0);
    }
    #[test]
    fn class_slots_validate_hash_and_right_finger_projection() {
        let bundle = embedded_phase4_bundle().unwrap();
        let mut parts = bundle.items.parts().clone();
        let row = parts.class_slots.first_mut().unwrap();
        row.m_def_slots_xml.insert(EquipSlot::Lfinger, 999);
        let changed = Phase4Rules::try_new(parts.clone()).unwrap();
        assert_ne!(changed.hash(), bundle.items.hash());
        parts
            .class_slots
            .first_mut()
            .unwrap()
            .m_def_slots
            .insert(EquipSlot::Lfinger, 999);
        assert!(Phase4Rules::try_new(parts).is_err());
        let mut parts = bundle.items.parts().clone();
        parts
            .class_slots
            .first_mut()
            .unwrap()
            .p_def_slots
            .remove(&EquipSlot::Chest);
        assert!(Phase4Rules::try_new(parts).is_err());
    }
}
