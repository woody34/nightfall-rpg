//! Equipment System-source ownership and armour-set listener history (D16/D20).
//! PcInventory.java:1147–1208,1232–1285; no timed effects or lifecycle callbacks.

use super::item_equipment::PaperdollMutation;
use super::item_inventory::InventoryChange;
use super::item_rules::{ArmorSetRule, ItemDefinition};
use super::phase4_checkpoint::{EquipmentSourceKey, SetActivation, SetSkillKind};
use super::{EquipSlot, ItemError, ItemInstance, Phase4Player, Phase4Rules};
use std::collections::{BTreeMap, BTreeSet};

/// Internal provider ordinal reserved for the set's direct base-attribute functions.
/// Skill ordinals retain their generated XML positions.
pub const SET_ATTRIBUTES_ORDINAL: u32 = u32::MAX;

fn equipped(player: &Phase4Player, slot: EquipSlot) -> Option<&ItemInstance> {
    player
        .paperdoll
        .get(&slot)
        .and_then(|id| player.inventory.get(id))
}
fn definition<'a>(
    item: &ItemInstance,
    rules: &'a Phase4Rules,
) -> Result<&'a ItemDefinition, ItemError> {
    rules
        .parts()
        .items
        .get(&item.template_id)
        .ok_or(ItemError("unknown equipped template"))
}
fn parts(set: &ArmorSetRule) -> [(EquipSlot, &[u32]); 5] {
    [
        (EquipSlot::Chest, &set.chest),
        (EquipSlot::Legs, &set.legs),
        (EquipSlot::Head, &set.head),
        (EquipSlot::Gloves, &set.gloves),
        (EquipSlot::Feet, &set.feet),
    ]
}
fn contains(set: &ArmorSetRule, item: &ItemInstance) -> bool {
    parts(set)
        .iter()
        .any(|(slot, ids)| item.slot == Some(*slot) && ids.contains(&item.template_id))
}
fn has_shield(player: &Phase4Player, set: &ArmorSetRule) -> bool {
    equipped(player, EquipSlot::Lhand).is_some_and(|item| set.shield.contains(&item.template_id))
}

/// Recompute membership while retaining the explicit shield history. A slot transition,
/// when supplied, is applied after its scalar mutation and before the next listener.
pub fn reconcile_sets(
    player: &Phase4Player,
    rules: &Phase4Rules,
    transition: Option<&InventoryChange>,
) -> Result<BTreeMap<u32, SetActivation>, ItemError> {
    let mut result = BTreeMap::new();
    let Some(chest) = equipped(player, EquipSlot::Chest) else {
        return Ok(result);
    };
    for set in rules
        .parts()
        .armor_sets
        .values()
        .filter(|set| set.chest.contains(&chest.template_id))
    {
        let full = parts(set).iter().all(|(slot, allowed)| {
            allowed.is_empty()
                || equipped(player, *slot).is_some_and(|item| allowed.contains(&item.template_id))
        });
        let enchant6 = full
            && parts(set).iter().all(|(slot, allowed)| {
                allowed.is_empty() || equipped(player, *slot).is_some_and(|item| item.enchant >= 6)
            });
        let mut shield = player
            .active_sets
            .get(&set.id)
            .filter(|s| s.chest == chest.id)
            .is_some_and(|s| s.shield_skill_active);
        if let Some(change) = transition {
            if let Some(before) = change.before.as_ref().filter(|before| {
                before.slot.is_some()
                    && change
                        .after
                        .as_ref()
                        .is_none_or(|after| after.slot != before.slot)
            }) {
                if contains(set, before) || set.shield.contains(&before.template_id) {
                    shield = false;
                }
            }
            if let Some(after) = change.after.as_ref().filter(|after| {
                after.slot.is_some()
                    && change
                        .before
                        .as_ref()
                        .is_none_or(|before| before.slot != after.slot)
            }) {
                if contains(set, after) {
                    if full && has_shield(player, set) {
                        shield = true;
                    }
                } else if set.shield.contains(&after.template_id) {
                    shield = true;
                }
            }
        }
        if shield && !has_shield(player, set) {
            return Err(ItemError("orphan set shield history"));
        }
        result.insert(
            set.id,
            SetActivation {
                chest: chest.id,
                full_set_active: full,
                shield_skill_active: shield,
                enchant6_active: enchant6,
            },
        );
    }
    Ok(result)
}

/// One D20 set-shield transition, observed for D46 `set_shield_history` evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs, clippy::struct_excessive_bools)] // The oracle's boolean inputs, verbatim.
pub struct SetShieldObservation {
    pub set_id: u32,
    pub previous: bool,
    pub mutation: &'static str,
    pub chest_present: bool,
    pub matching_shield: bool,
    pub complete: bool,
    pub required_part: bool,
    pub active: bool,
}

/// Replays a paperdoll mutation's slot transitions in listener order (as `apply_paperdoll`
/// does) and reports, for each transition touching a set part or set shield of the worn
/// chest's sets, the shield history before and after. Evidence only; never mutates.
pub fn set_shield_observations(
    player: &Phase4Player,
    mutation: &PaperdollMutation,
    rules: &Phase4Rules,
) -> Result<Vec<SetShieldObservation>, ItemError> {
    let mut staged = player.clone();
    let mut out = Vec::new();
    for change in &mutation.changes {
        let previous_sets = staged.active_sets.clone();
        let instance = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .ok_or(ItemError("empty inventory transition"))?
            .id;
        if let Some(slot) = change.before.as_ref().and_then(|before| before.slot) {
            staged.paperdoll.remove(&slot);
        }
        if let Some(after) = &change.after {
            if let Some(slot) = after.slot {
                staged.paperdoll.insert(slot, after.id);
            }
            staged.inventory.insert(instance, after.clone());
        } else {
            staged.inventory.remove(&instance);
        }
        let after_sets = reconcile_sets(&staged, rules, Some(change))?;
        let removed = change.before.as_ref().filter(|before| {
            before.slot.is_some()
                && change
                    .after
                    .as_ref()
                    .is_none_or(|after| after.slot != before.slot)
        });
        let added = change.after.as_ref().filter(|after| {
            after.slot.is_some()
                && change
                    .before
                    .as_ref()
                    .is_none_or(|before| before.slot != after.slot)
        });
        if let Some(chest) = equipped(&staged, EquipSlot::Chest) {
            for set in rules
                .parts()
                .armor_sets
                .values()
                .filter(|set| set.chest.contains(&chest.template_id))
            {
                let (kind, required_part) = if let Some(before) = removed {
                    if !contains(set, before) && !set.shield.contains(&before.template_id) {
                        continue;
                    }
                    ("remove", contains(set, before))
                } else if let Some(after) = added {
                    if set.shield.contains(&after.template_id) {
                        ("equip_shield", false)
                    } else if contains(set, after) {
                        ("equip_chest", false)
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                let complete = parts(set).iter().all(|(slot, allowed)| {
                    allowed.is_empty()
                        || equipped(&staged, *slot)
                            .is_some_and(|item| allowed.contains(&item.template_id))
                });
                out.push(SetShieldObservation {
                    set_id: set.id,
                    previous: previous_sets
                        .get(&set.id)
                        .filter(|s| s.chest == chest.id)
                        .is_some_and(|s| s.shield_skill_active),
                    mutation: kind,
                    chest_present: true,
                    matching_shield: has_shield(&staged, set),
                    complete,
                    required_part,
                    active: after_sets
                        .get(&set.id)
                        .is_some_and(|s| s.shield_skill_active),
                });
            }
        }
        staged.active_sets = after_sets;
    }
    Ok(out)
}

/// Catalogue-derived provider identities. Reconciliation allocates only genuinely new
/// providers, preserving enchant refreshes and death/resurrection identity exactly.
pub fn desired_sources(
    player: &Phase4Player,
    rules: &Phase4Rules,
) -> Result<BTreeSet<EquipmentSourceKey>, ItemError> {
    let mut desired = BTreeSet::new();
    for id in player.paperdoll.values() {
        let item = player
            .inventory
            .get(id)
            .ok_or(ItemError("missing equipped instance"))?;
        let def = definition(item, rules)?;
        desired.insert(EquipmentSourceKey::Item { instance: *id });
        for skill in &def.skills {
            desired.insert(EquipmentSourceKey::ItemSkill {
                instance: *id,
                xml_ordinal: skill.xml_ordinal,
            });
        }
        if item.enchant >= 4 && !def.enchant4_skills.is_empty() {
            desired.insert(EquipmentSourceKey::Enchant4 { instance: *id });
        }
        if let Some(levels) = rules.parts().enchant_options.get(&item.template_id) {
            for level in levels.iter().filter(|level| level.level == item.enchant) {
                for option in &level.option_ids {
                    desired.insert(EquipmentSourceKey::EnchantOption {
                        instance: *id,
                        option: *option,
                    });
                }
            }
        }
        if let Some(packed) = item.augment_id {
            for (half, option) in [(0, packed & 0xffff), (1, packed >> 16)] {
                if !rules
                    .parts()
                    .option_capabilities
                    .get(&option)
                    .is_some_and(|cap| cap.admitted)
                {
                    return Err(ItemError("deferred augmentation option"));
                }
                desired.insert(EquipmentSourceKey::Augment {
                    instance: *id,
                    half,
                });
            }
        }
        if item.slot == Some(EquipSlot::Rhand) && def.weapon_type == "POLE" {
            desired.insert(EquipmentSourceKey::Polearm);
        }
    }
    for (id, active) in &player.active_sets {
        let set = rules
            .parts()
            .armor_sets
            .get(id)
            .ok_or(ItemError("unknown active set"))?;
        for (enabled, kind, skills) in [
            (active.full_set_active, SetSkillKind::Full, &set.skills),
            (active.shield_skill_active, SetSkillKind::Shield, &set.shield_skills),
            (active.enchant6_active, SetSkillKind::Enchant6, &set.enchant6_skills),
        ] {
            if enabled {
                for skill in skills {
                    desired.insert(EquipmentSourceKey::ArmorSet {
                        chest: active.chest,
                        set: *id,
                        skill_kind: kind,
                        ordinal: skill.xml_ordinal,
                    });
                }
            }
        }
        if active.full_set_active && !set.base_stats.is_empty() {
            desired.insert(EquipmentSourceKey::ArmorSet {
                chest: active.chest,
                set: *id,
                skill_kind: SetSkillKind::Full,
                ordinal: SET_ATTRIBUTES_ORDINAL,
            });
        }
    }
    for (weapon, rank) in [(true, player.weapon_penalty), (false, player.armor_penalty)] {
        if rank > 0 {
            desired.insert(EquipmentSourceKey::GradePenalty { weapon });
        }
    }
    if player.overload_level > 0 {
        desired.insert(EquipmentSourceKey::Overload);
    }
    Ok(desired)
}

/// Update a private staged player and cursor; callers commit both only after success.
pub fn reconcile_sources(
    player: &mut Phase4Player,
    rules: &Phase4Rules,
    next_sequence: &mut u64,
) -> Result<(), ItemError> {
    let desired = desired_sources(player, rules)?;
    let mut sources = player.equipment_sources.clone();
    sources.retain(|_, source| desired.contains(source));
    let mut seen = BTreeSet::new();
    for (sequence, source) in &sources {
        if *sequence == 0 || *sequence >= *next_sequence || !seen.insert(source.clone()) {
            return Err(ItemError("corrupt equipment install order"));
        }
    }
    let mut next = *next_sequence;
    for source in desired.difference(&seen) {
        if next == 0 {
            return Err(ItemError("zero provider cursor"));
        }
        let sequence = next;
        next = next
            .checked_add(1)
            .ok_or(ItemError("equipment provider cursor overflow"))?;
        sources.insert(sequence, source.clone());
    }
    player.equipment_sources = sources;
    *next_sequence = next;
    Ok(())
}

/// Apply every removal/installation in source listener order on a private clone.
/// Intermediate states remain private; malformed plans and allocation exhaustion roll back.
pub fn apply_paperdoll(
    player: &Phase4Player,
    mutation: &PaperdollMutation,
    rules: &Phase4Rules,
    next_sequence: u64,
) -> Result<(Phase4Player, u64), ItemError> {
    let mut staged = player.clone();
    let mut next = next_sequence;
    for change in &mutation.changes {
        let instance = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .ok_or(ItemError("empty inventory transition"))?
            .id;
        if staged.inventory.get(&instance) != change.before.as_ref()
            || change
                .after
                .as_ref()
                .is_some_and(|after| after.id != instance)
            || change
                .before
                .as_ref()
                .is_some_and(|before| before.id != instance)
        {
            return Err(ItemError("equipment transition does not match inventory"));
        }
        if let Some(slot) = change.before.as_ref().and_then(|before| before.slot) {
            staged.paperdoll.remove(&slot);
        }
        if let Some(after) = &change.after {
            if let Some(slot) = after.slot {
                staged.paperdoll.insert(slot, after.id);
            }
            staged.inventory.insert(instance, after.clone());
        } else {
            staged.inventory.remove(&instance);
        }
        staged.active_sets = reconcile_sets(&staged, rules, Some(change))?;
        reconcile_sources(&mut staged, rules, &mut next)?;
    }
    if staged.inventory != mutation.inventory || staged.paperdoll != mutation.paperdoll {
        return Err(ItemError("equipment transition projection mismatch"));
    }
    Ok((staged, next))
}

/// Source overload rank and historical movement flag. L2PcInstance.java:1668–1693
/// updates the flag only when the rank changes; a nonpositive maximum is a no-op.
pub fn overload_state(
    weight: i64,
    max_load: i64,
    bonus: i64,
    previous: (u8, bool),
    rules: &Phase4Rules,
) -> Result<(u8, bool), ItemError> {
    if weight < 0 || previous.0 > 4 {
        return Err(ItemError("invalid overload scalar"));
    }
    if max_load <= 0 {
        return Ok(previous);
    }
    let per_mille = i128::from(weight)
        .checked_sub(i128::from(bonus))
        .and_then(|v| v.checked_mul(1000))
        .and_then(|v| v.checked_div(i128::from(max_load)))
        .ok_or(ItemError("overload arithmetic overflow"))?;
    let rank = u8::try_from(
        rules
            .parts()
            .capacities
            .overload_thresholds
            .iter()
            .take_while(|threshold| per_mille >= i128::from(**threshold))
            .count(),
    )
    .map_err(|_| ItemError("overload rank overflow"))?;
    Ok(if rank == previous.0 {
        previous
    } else {
        (rank, rank > 0 && weight > max_load)
    })
}

/// Refresh scalar penalties after the preceding attribute/load fold. Provider refresh is
/// separate so a whole command can stage its fold, source identities and cursor together.
pub fn refresh_penalties(
    player: &mut Phase4Player,
    rules: &Phase4Rules,
    expertise_bonus: i64,
) -> Result<(), ItemError> {
    let (_, _, weight) = super::item_inventory::inventory_totals(&player.inventory, rules)?;
    let mut weapon = 0_u8;
    let mut armor = 0_u8;
    for item in player.inventory.values().filter(|item| item.slot.is_some()) {
        let def = definition(item, rules)?;
        if matches!(def.etcitem_type.as_str(), "ARROW" | "BOLT") {
            continue;
        }
        let grade = rules
            .parts()
            .grades
            .iter()
            .find(|row| row.grade == def.grade)
            .ok_or(ItemError("unknown equipped grade"))?
            .id;
        if grade > player.expertise {
            // Preserve the source weapon-to-armour else-if branch, even with malformed
            // multiple-weapon input; ownership/slot validation remains the caller's gate.
            if def.kind == "Weapon" && grade > weapon {
                weapon = grade;
            } else if grade > armor {
                armor = grade;
            }
        }
    }
    let penalty = |grade: u8| -> Result<u8, ItemError> {
        let value = i128::from(grade)
            .checked_sub(i128::from(player.expertise))
            .and_then(|v| v.checked_sub(i128::from(expertise_bonus)))
            .ok_or(ItemError("grade penalty overflow"))?;
        u8::try_from(value.clamp(0, 4)).map_err(|_| ItemError("grade penalty overflow"))
    };
    let weapon = penalty(weapon)?;
    let armor = penalty(armor)?;
    let (rank, overloaded) = overload_state(
        weight,
        player.max_load,
        player.weight_penalty_bonus,
        (player.overload_level, player.overloaded),
        rules,
    )?;
    player.weight = weight;
    player.weapon_penalty = weapon;
    player.armor_penalty = armor;
    player.overload_level = rank;
    player.overloaded = overloaded;
    Ok(())
}

/// Resolve the skills owned by one validated System provider. Active skills remain grants;
/// only passive definitions enter the stat fold. Trigger skills retain their own paths.
pub fn source_skills(
    player: &Phase4Player,
    rules: &Phase4Rules,
    source: &EquipmentSourceKey,
) -> Result<Vec<super::skill::SkillRef>, ItemError> {
    use super::skill::{SkillId, SkillRef};
    let item = |id| {
        player
            .inventory
            .get(id)
            .ok_or(ItemError("missing provider item"))
    };
    let selected: Vec<&super::item_rules::ItemSkillRef> = match source {
        EquipmentSourceKey::Item { .. } => Vec::new(),
        EquipmentSourceKey::ItemSkill {
            instance,
            xml_ordinal,
        } => vec![definition(item(instance)?, rules)?
            .skills
            .iter()
            .find(|skill| skill.xml_ordinal == *xml_ordinal)
            .ok_or(ItemError("missing item skill ordinal"))?],
        EquipmentSourceKey::Enchant4 { instance } => definition(item(instance)?, rules)?
            .enchant4_skills
            .iter()
            .collect(),
        EquipmentSourceKey::EnchantOption { .. } | EquipmentSourceKey::Augment { .. } => {
            source_option(player, rules, source)?
                .ok_or(ItemError("missing option provider"))?
                .skills
                .iter()
                .filter(|skill| !matches!(skill.kind.as_str(), "attack_skill" | "magic_skill"))
                .collect()
        },
        EquipmentSourceKey::ArmorSet {
            set,
            skill_kind,
            ordinal,
            ..
        } => {
            if *ordinal == SET_ATTRIBUTES_ORDINAL {
                return Ok(Vec::new());
            }
            let set = rules
                .parts()
                .armor_sets
                .get(set)
                .ok_or(ItemError("missing set provider"))?;
            let skills = match skill_kind {
                SetSkillKind::Full => &set.skills,
                SetSkillKind::Shield => &set.shield_skills,
                SetSkillKind::Enchant6 => &set.enchant6_skills,
            };
            vec![skills
                .iter()
                .find(|skill| skill.xml_ordinal == *ordinal)
                .ok_or(ItemError("missing set skill ordinal"))?]
        },
        EquipmentSourceKey::GradePenalty { weapon } => {
            return Ok(vec![SkillRef {
                id: SkillId(if *weapon { 6209 } else { 6213 }),
                rank: u32::from(if *weapon {
                    player.weapon_penalty
                } else {
                    player.armor_penalty
                }),
            }])
        },
        EquipmentSourceKey::Overload => {
            return Ok(vec![SkillRef {
                id: SkillId(4270),
                rank: u32::from(player.overload_level),
            }])
        },
        EquipmentSourceKey::Polearm => {
            return Ok(vec![SkillRef {
                id: SkillId(3599),
                rank: 1,
            }])
        },
    };
    Ok(selected
        .into_iter()
        .map(|skill| SkillRef {
            id: SkillId(skill.id),
            rank: skill.level,
        })
        .collect())
}

/// Exact option half, shared by passive functions and capability/proc resolution.
pub fn source_option<'a>(
    player: &Phase4Player,
    rules: &'a Phase4Rules,
    source: &EquipmentSourceKey,
) -> Result<Option<&'a super::item_rules::AugmentOptionRule>, ItemError> {
    let option = match source {
        EquipmentSourceKey::EnchantOption { option, .. } => *option,
        EquipmentSourceKey::Augment { instance, half } => {
            let packed = player
                .inventory
                .get(instance)
                .and_then(|item| item.augment_id)
                .ok_or(ItemError("missing augmentation"))?;
            match half {
                0 => packed & 0xffff,
                1 => packed >> 16,
                _ => return Err(ItemError("invalid option half")),
            }
        },
        EquipmentSourceKey::Item { .. }
        | EquipmentSourceKey::ItemSkill { .. }
        | EquipmentSourceKey::Enchant4 { .. }
        | EquipmentSourceKey::ArmorSet { .. }
        | EquipmentSourceKey::GradePenalty { .. }
        | EquipmentSourceKey::Overload
        | EquipmentSourceKey::Polearm => return Ok(None),
    };
    rules
        .parts()
        .augment_options
        .get(&option)
        .map(Some)
        .ok_or(ItemError("missing option definition"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::item_functions::tests::{equip, player};
    use super::super::{ItemInstanceId, ItemLocation};
    use super::*;
    use crate::infrastructure::item_data::embedded_phase4_bundle;

    fn put(player: &Phase4Player, id: u64, rules: &Phase4Rules, next: u64) -> (Phase4Player, u64) {
        let mutation = PaperdollMutation::equip(
            &player.inventory,
            &player.paperdoll,
            ItemInstanceId::new(id).unwrap(),
            None,
            rules,
            2,
        )
        .unwrap();
        apply_paperdoll(player, &mutation, rules, next).unwrap()
    }
    #[test]
    fn incomplete_set_shield_history_follows_slot_order_and_survives_rebuild() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let set = rules
            .parts()
            .armor_sets
            .values()
            .find(|set| {
                !set.shield.is_empty()
                    && !set.legs.is_empty()
                    && !set.head.is_empty()
                    && parts(set)
                        .iter()
                        .flat_map(|(_, ids)| ids.iter())
                        .chain(&set.shield)
                        .all(|id| {
                            rules
                                .parts()
                                .capabilities
                                .get(id)
                                .is_some_and(|cap| cap.admitted)
                        })
            })
            .unwrap();
        let mut original = player();
        equip(&mut original, *set.chest.first().unwrap(), 0, EquipSlot::Chest, 1);
        equip(&mut original, *set.shield.first().unwrap(), 0, EquipSlot::Lhand, 2);
        equip(&mut original, *set.legs.first().unwrap(), 0, EquipSlot::Legs, 3);
        for item in original.inventory.values_mut() {
            item.slot = None;
            item.location = ItemLocation::Inventory;
        }
        original.paperdoll.clear();
        original.equipment_sources.clear();
        let (first, next) = put(&original, 2, rules, 1);
        let (shield_first, _) = put(&first, 1, rules, next);
        assert!(
            !shield_first
                .active_sets
                .get(&set.id)
                .unwrap()
                .shield_skill_active
        );
        let (first, next) = put(&original, 1, rules, 1);
        let (chest_first, next) = put(&first, 2, rules, next);
        let active = chest_first.active_sets.get(&set.id).unwrap();
        assert!(active.shield_skill_active);
        assert!(!active.full_set_active);
        let restored: Phase4Player =
            serde_json::from_slice(&serde_json::to_vec(&chest_first).unwrap()).unwrap();
        assert_eq!(reconcile_sets(&restored, rules, None).unwrap(), chest_first.active_sets);
        let mut rebuilt = restored.clone();
        let mut rebuilt_next = next;
        reconcile_sources(&mut rebuilt, rules, &mut rebuilt_next).unwrap();
        assert_eq!(rebuilt, restored);
        assert_eq!(rebuilt_next, next);
        let (with_legs, next) = put(&restored, 3, rules, next);
        let mutation = PaperdollMutation::unequip(
            &with_legs.inventory,
            &with_legs.paperdoll,
            EquipSlot::Legs,
            rules,
            2,
        )
        .unwrap();
        let (removed, _) = apply_paperdoll(&with_legs, &mutation, rules, next).unwrap();
        assert!(
            !removed
                .active_sets
                .get(&set.id)
                .unwrap()
                .shield_skill_active
        );
        assert!(
            chest_first
                .active_sets
                .get(&set.id)
                .unwrap()
                .shield_skill_active
        );
    }
    #[test]
    fn overload_thresholds_and_same_rank_history_are_exact() {
        let bundle = embedded_phase4_bundle().unwrap();
        for (weight, rank) in [
            (500, 0),
            (501, 1),
            (666, 1),
            (667, 2),
            (800, 2),
            (801, 3),
            (1000, 3),
            (1001, 4),
        ] {
            assert_eq!(
                overload_state(weight, 1000, 1, (0, false), &bundle.items).unwrap(),
                (rank, rank == 4)
            );
        }
        assert_eq!(overload_state(1001, 1000, 0, (4, false), &bundle.items).unwrap(), (4, false));
        assert_eq!(overload_state(1000, 1000, 0, (4, true), &bundle.items).unwrap(), (4, true));
        assert_eq!(overload_state(999, 1000, 0, (4, true), &bundle.items).unwrap(), (3, false));
        assert_eq!(overload_state(0, 0, 1, (4, true), &bundle.items).unwrap(), (4, true));
        assert_eq!(overload_state(0, 1000, 1, (4, true), &bundle.items).unwrap(), (0, false));
    }
    #[test]
    fn replacement_reallocates_only_removed_sources_and_cursor_failure_is_atomic() {
        let bundle = embedded_phase4_bundle().unwrap();
        let mut original = player();
        equip(&mut original, 2369, 0, EquipSlot::Rhand, 1);
        equip(&mut original, 1146, 0, EquipSlot::Chest, 2);
        let mutation = PaperdollMutation::unequip(
            &original.inventory,
            &original.paperdoll,
            EquipSlot::Rhand,
            &bundle.items,
            2,
        )
        .unwrap();
        let (removed, next) = apply_paperdoll(&original, &mutation, &bundle.items, 3).unwrap();
        assert_eq!(next, 3);
        assert!(removed.equipment_sources.contains_key(&2));
        let restored = PaperdollMutation::equip(
            &removed.inventory,
            &removed.paperdoll,
            ItemInstanceId::new(1).unwrap(),
            None,
            &bundle.items,
            2,
        )
        .unwrap();
        assert!(apply_paperdoll(&removed, &restored, &bundle.items, u64::MAX).is_err());
        let (again, next) = apply_paperdoll(&removed, &restored, &bundle.items, next).unwrap();
        assert_eq!(next, 4);
        assert_eq!(again.equipment_sources.get(&2), original.equipment_sources.get(&2));
        assert_eq!(again.equipment_sources.get(&3), original.equipment_sources.get(&1));
        assert_eq!(removed.equipment_sources.len(), 1);
    }
}
