//! Atomic paperdoll scalar plans. The caller validates actor/template conditions and folds
//! each ordered removal/installation into the shared provider sequence before publishing.
use super::item_inventory::{inventory_totals, InventoryChange};
use super::item_rules::ItemDefinition;
use super::{EquipSlot, ItemError, ItemInstance, ItemInstanceId, ItemLocation, Phase4Rules};
use std::collections::{BTreeMap, BTreeSet};

/// Complete staged scalar state plus source listener order for provider reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct PaperdollMutation {
    pub inventory: BTreeMap<ItemInstanceId, ItemInstance>,
    pub paperdoll: BTreeMap<EquipSlot, ItemInstanceId>,
    pub changes: Vec<InventoryChange>,
}
impl PaperdollMutation {
    fn stage(
        inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
        paperdoll: &BTreeMap<EquipSlot, ItemInstanceId>,
        rules: &Phase4Rules,
        revision: u64,
    ) -> Result<Self, ItemError> {
        inventory_totals(inventory, rules)?;
        validate_paperdoll(inventory, paperdoll, rules)?;
        if i64::try_from(revision).is_err() || inventory.values().any(|i| i.revision > revision) {
            return Err(ItemError("invalid equipment revision"));
        }
        Ok(Self {
            inventory: inventory.clone(),
            paperdoll: paperdoll.clone(),
            changes: Vec::new(),
        })
    }
    /// Equip with source paired-slot preference; an explicit slot may select only a
    /// compatible paired location. Grade mismatch is deliberately not an equip refusal.
    pub fn equip(
        inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
        paperdoll: &BTreeMap<EquipSlot, ItemInstanceId>,
        instance: ItemInstanceId,
        preferred: Option<EquipSlot>,
        rules: &Phase4Rules,
        revision: u64,
    ) -> Result<Self, ItemError> {
        let mut plan = Self::stage(inventory, paperdoll, rules, revision)?;
        let item = inventory
            .get(&instance)
            .ok_or(ItemError("item is not owned"))?;
        let definition = definition(item, rules)?;
        if !rules
            .parts()
            .capabilities
            .get(&item.template_id)
            .is_some_and(|cap| cap.admitted)
        {
            return Err(ItemError("equipment capability deferred"));
        }
        if let Some(slot) = item.slot {
            if preferred.is_none_or(|requested| requested == slot) {
                return Ok(plan);
            }
        }
        let slot = choose_slot(definition, preferred, paperdoll)?;
        if plan
            .at(EquipSlot::Chest, rules)?
            .is_some_and(|d| d.bodypart == "alldress")
            && item.template_id != 21163
            && matches!(
                slot,
                EquipSlot::Rhand
                    | EquipSlot::Lhand
                    | EquipSlot::Legs
                    | EquipSlot::Feet
                    | EquipSlot::Gloves
                    | EquipSlot::Head
            )
        {
            return Err(ItemError("formal wear blocks equipment slot"));
        }
        if let Some(previous) = item.slot {
            plan.set(previous, None, rules, revision)?;
        }
        match definition.bodypart.as_str() {
            "lrhand" => plan.set(EquipSlot::Lhand, None, rules, revision)?,
            "lhand" => {
                if plan
                    .at(EquipSlot::Rhand, rules)?
                    .is_some_and(|r| r.bodypart == "lrhand" && !ammunition_pair(r, definition))
                {
                    plan.set(EquipSlot::Rhand, None, rules, revision)?;
                }
            },
            "onepiece" => plan.set(EquipSlot::Legs, None, rules, revision)?,
            "legs" => {
                if plan
                    .at(EquipSlot::Chest, rules)?
                    .is_some_and(|d| d.bodypart == "onepiece")
                {
                    plan.set(EquipSlot::Chest, None, rules, revision)?;
                }
            },
            "hairall" => plan.set(EquipSlot::Hair2, None, rules, revision)?,
            "hair2" => {
                if plan
                    .at(EquipSlot::Hair, rules)?
                    .is_some_and(|d| d.bodypart == "hairall")
                {
                    plan.set(EquipSlot::Hair, None, rules, revision)?;
                }
            },
            "alldress" => {
                for displaced in [
                    EquipSlot::Legs,
                    EquipSlot::Lhand,
                    EquipSlot::Rhand,
                    EquipSlot::Head,
                    EquipSlot::Feet,
                    EquipSlot::Gloves,
                ] {
                    plan.set(displaced, None, rules, revision)?;
                }
            },
            _ => {},
        }
        plan.set(slot, Some(instance), rules, revision)?;
        validate_paperdoll(&plan.inventory, &plan.paperdoll, rules)?;
        Ok(plan)
    }
    /// Removing a bow/crossbow also removes its ammunition; charge bits clear on removal.
    pub fn unequip(
        inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
        paperdoll: &BTreeMap<EquipSlot, ItemInstanceId>,
        slot: EquipSlot,
        rules: &Phase4Rules,
        revision: u64,
    ) -> Result<Self, ItemError> {
        let mut plan = Self::stage(inventory, paperdoll, rules, revision)?;
        plan.set(slot, None, rules, revision)?;
        validate_paperdoll(&plan.inventory, &plan.paperdoll, rules)?;
        Ok(plan)
    }
    /// checkAndEquipArrows (L2PcInstance.java:5334-5352): `setPaperdollItem(LHAND, arrow)`
    /// into an empty left hand, bypassing equipItem's displacement rules (D85).
    pub fn install_ammunition(
        inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
        paperdoll: &BTreeMap<EquipSlot, ItemInstanceId>,
        ammunition: ItemInstanceId,
        rules: &Phase4Rules,
        revision: u64,
    ) -> Result<Self, ItemError> {
        if paperdoll.contains_key(&EquipSlot::Lhand) {
            return Err(ItemError("left hand already holds ammunition"));
        }
        let mut plan = Self::stage(inventory, paperdoll, rules, revision)?;
        plan.set(EquipSlot::Lhand, Some(ammunition), rules, revision)?;
        validate_paperdoll(&plan.inventory, &plan.paperdoll, rules)?;
        Ok(plan)
    }
    fn at<'a>(
        &self,
        slot: EquipSlot,
        rules: &'a Phase4Rules,
    ) -> Result<Option<&'a ItemDefinition>, ItemError> {
        self.paperdoll
            .get(&slot)
            .map(|id| {
                self.inventory
                    .get(id)
                    .ok_or(ItemError("paperdoll item absent"))
                    .and_then(|item| definition(item, rules))
            })
            .transpose()
    }
    fn set(
        &mut self,
        slot: EquipSlot,
        next: Option<ItemInstanceId>,
        rules: &Phase4Rules,
        revision: u64,
    ) -> Result<(), ItemError> {
        if self.paperdoll.get(&slot).copied() == next {
            return Ok(());
        }
        if let Some(id) = self.paperdoll.remove(&slot) {
            let before = self
                .inventory
                .get(&id)
                .ok_or(ItemError("paperdoll item absent"))?
                .clone();
            let def = definition(&before, rules)?;
            let removes_ammunition = slot == EquipSlot::Rhand
                && matches!(def.weapon_type.as_str(), "BOW" | "CROSSBOW" | "FISHINGROD");
            let mut after = before.clone();
            after.location = ItemLocation::Inventory;
            after.slot = None;
            after.charges = 0;
            after.revision = revision;
            self.inventory.insert(id, after.clone());
            self.changes.push(InventoryChange {
                before: Some(before),
                after: Some(after),
            });
            if removes_ammunition {
                self.set(EquipSlot::Lhand, None, rules, revision)?;
            }
        }
        if let Some(id) = next {
            let before = self
                .inventory
                .get(&id)
                .ok_or(ItemError("equipment item absent"))?
                .clone();
            if before.slot.is_some() {
                return Err(ItemError("item already occupies a slot"));
            }
            let def = definition(&before, rules)?;
            let ammo_type = if slot == EquipSlot::Rhand {
                match def.weapon_type.as_str() {
                    "BOW" => Some("ARROW"),
                    "CROSSBOW" => Some("BOLT"),
                    _ => None,
                }
            } else {
                None
            };
            let mut after = before.clone();
            after.location = ItemLocation::Paperdoll;
            after.slot = Some(slot);
            after.revision = revision;
            self.inventory.insert(id, after.clone());
            self.paperdoll.insert(slot, id);
            self.changes.push(InventoryChange {
                before: Some(before),
                after: Some(after),
            });
            if let Some(kind) = ammo_type {
                let grade = rules
                    .parts()
                    .grades
                    .iter()
                    .find(|grade| grade.grade == def.grade)
                    .ok_or(ItemError("weapon grade absent"))?;
                let mut selected = None;
                for item in self.inventory.values() {
                    let ammo = definition(item, rules)?;
                    if ammo.etcitem_type == kind
                        && rules
                            .parts()
                            .grades
                            .iter()
                            .find(|grade| grade.grade == ammo.grade)
                            .is_some_and(|g| g.shot_grade == grade.shot_grade)
                    {
                        selected = Some(item.id);
                        break;
                    }
                }
                if let Some(ammo) = selected {
                    self.set(EquipSlot::Lhand, Some(ammo), rules, revision)?;
                }
            }
        }
        Ok(())
    }
}
fn definition<'a>(
    item: &ItemInstance,
    rules: &'a Phase4Rules,
) -> Result<&'a ItemDefinition, ItemError> {
    rules
        .parts()
        .items
        .get(&item.template_id)
        .ok_or(ItemError("unknown equipment template"))
}
/// Inventory.equipItem LHAND displacement pairs (Inventory.java:560-568).
fn ammunition_pair(right: &ItemDefinition, left: &ItemDefinition) -> bool {
    matches!(
        (right.weapon_type.as_str(), left.etcitem_type.as_str()),
        ("BOW", "ARROW") | ("CROSSBOW", "BOLT") | ("FISHINGROD", "LURE")
    )
}
/// Legal held pairs: the equip pairs plus a crossbow holding arrows, which the attack-time
/// refill installs without displacement (checkAndEquipArrows, L2PcInstance.java:5334-5352; D85).
fn held_ammunition(right: &ItemDefinition, left: &ItemDefinition) -> bool {
    ammunition_pair(right, left)
        || (right.weapon_type == "CROSSBOW" && left.etcitem_type == "ARROW")
}
fn slots(bodypart: &str) -> Result<&'static [EquipSlot], ItemError> {
    use EquipSlot::{
        Belt, Chest, Cloak, Feet, Gloves, Hair, Hair2, Head, Lbracelet, Lear, Legs, Lfinger, Lhand,
        Neck, Rbracelet, Rear, Rfinger, Rhand, Under,
    };
    Ok(match bodypart {
        "rhand" | "lrhand" => &[Rhand],
        "lhand" => &[Lhand],
        "chest" | "onepiece" | "alldress" => &[Chest],
        "legs" => &[Legs],
        "head" => &[Head],
        "feet" => &[Feet],
        "gloves" => &[Gloves],
        "neck" => &[Neck],
        "underwear" => &[Under],
        "hair" | "hairall" => &[Hair],
        "hair2" => &[Hair2],
        "back" => &[Cloak],
        "waist" => &[Belt],
        "lbracelet" => &[Lbracelet],
        "rbracelet" => &[Rbracelet],
        "lear" | "rear" | "rear;lear" => &[Lear, Rear],
        "lfinger" | "rfinger" | "rfinger;lfinger" => &[Lfinger, Rfinger],
        _ => return Err(ItemError("bodypart cannot be equipped")),
    })
}
fn choose_slot(
    def: &ItemDefinition,
    preferred: Option<EquipSlot>,
    paperdoll: &BTreeMap<EquipSlot, ItemInstanceId>,
) -> Result<EquipSlot, ItemError> {
    let compatible = slots(&def.bodypart)?;
    if let Some(slot) = preferred {
        if !compatible.contains(&slot)
            || (def.bodypart == "lear" && slot != EquipSlot::Lear)
            || (def.bodypart == "rear" && slot != EquipSlot::Rear)
            || (def.bodypart == "lfinger" && slot != EquipSlot::Lfinger)
            || (def.bodypart == "rfinger" && slot != EquipSlot::Rfinger)
        {
            return Err(ItemError("incompatible preferred slot"));
        }
        return Ok(slot);
    }
    compatible
        .iter()
        .find(|slot| !paperdoll.contains_key(slot))
        .or(compatible.first())
        .copied()
        .ok_or(ItemError("empty compatible slots"))
}
/// Bijection and source conflict validation; live conditions/history are checked separately.
pub fn validate_paperdoll(
    inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
    paperdoll: &BTreeMap<EquipSlot, ItemInstanceId>,
    rules: &Phase4Rules,
) -> Result<(), ItemError> {
    let mut seen = BTreeSet::new();
    for (slot, id) in paperdoll {
        let item = inventory
            .get(id)
            .ok_or(ItemError("paperdoll item absent"))?;
        let def = definition(item, rules)?;
        if !seen.insert(*id)
            || item.slot != Some(*slot)
            || item.location != ItemLocation::Paperdoll
            || !slots(&def.bodypart)?.contains(slot)
        {
            return Err(ItemError("invalid paperdoll index"));
        }
    }
    if inventory
        .values()
        .any(|item| item.slot.is_some() != seen.contains(&item.id))
    {
        return Err(ItemError("orphan equipment scalar"));
    }
    let at = |slot| {
        paperdoll
            .get(&slot)
            .and_then(|id| inventory.get(id))
            .map(|item| definition(item, rules))
            .transpose()
    };
    if let (Some(right), Some(left)) = (at(EquipSlot::Rhand)?, at(EquipSlot::Lhand)?) {
        if right.bodypart == "lrhand" && !held_ammunition(right, left) {
            return Err(ItemError("two-hand equipment conflict"));
        }
    }
    if at(EquipSlot::Chest)?.is_some_and(|d| d.bodypart == "onepiece")
        && paperdoll.contains_key(&EquipSlot::Legs)
    {
        return Err(ItemError("full armor equipment conflict"));
    }
    if at(EquipSlot::Hair)?.is_some_and(|d| d.bodypart == "hairall")
        && paperdoll.contains_key(&EquipSlot::Hair2)
    {
        return Err(ItemError("full hair equipment conflict"));
    }
    if at(EquipSlot::Chest)?.is_some_and(|d| d.bodypart == "alldress") {
        for slot in [
            EquipSlot::Rhand,
            EquipSlot::Lhand,
            EquipSlot::Legs,
            EquipSlot::Feet,
            EquipSlot::Gloves,
            EquipSlot::Head,
        ] {
            if at(slot)?.is_some_and(|d| d.template_id != 21163) {
                return Err(ItemError("formal wear equipment conflict"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::domain::zone::Tick;
    use crate::infrastructure::item_data::embedded_phase4_bundle;
    fn items(templates: &[u32]) -> BTreeMap<ItemInstanceId, ItemInstance> {
        templates
            .iter()
            .enumerate()
            .map(|(index, template)| {
                let id = ItemInstanceId::new(u64::try_from(index).unwrap().checked_add(1).unwrap())
                    .unwrap();
                (
                    id,
                    ItemInstance {
                        creation_origin: None,
                        id,
                        template_id: *template,
                        count: 1,
                        location: ItemLocation::Inventory,
                        slot: None,
                        enchant: 0,
                        augment_id: None,
                        charges: 0,
                        created_tick: Tick(0),
                        revision: 1,
                    },
                )
            })
            .collect()
    }
    fn id(value: u64) -> ItemInstanceId {
        ItemInstanceId::new(value).unwrap()
    }
    #[test]
    fn paired_slots_follow_source_preference_allow_explicit_side_and_remove_before_install() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        for (template, left, right) in [
            (112, EquipSlot::Lear, EquipSlot::Rear),
            (116, EquipSlot::Lfinger, EquipSlot::Rfinger),
        ] {
            let inventory = items(&[template, template, template]);
            let first =
                PaperdollMutation::equip(&inventory, &BTreeMap::new(), id(1), None, rules, 2)
                    .unwrap();
            assert_eq!(*first.paperdoll.get(&left).unwrap(), id(1));
            let second =
                PaperdollMutation::equip(&first.inventory, &first.paperdoll, id(2), None, rules, 2)
                    .unwrap();
            assert_eq!(*second.paperdoll.get(&right).unwrap(), id(2));
            let third = PaperdollMutation::equip(
                &second.inventory,
                &second.paperdoll,
                id(3),
                None,
                rules,
                2,
            )
            .unwrap();
            assert_eq!(*third.paperdoll.get(&left).unwrap(), id(3));
            assert_eq!(
                third
                    .changes
                    .iter()
                    .map(|c| c.after.as_ref().unwrap().slot)
                    .collect::<Vec<_>>(),
                vec![None, Some(left)]
            );
            let moved = PaperdollMutation::equip(
                &third.inventory,
                &third.paperdoll,
                id(3),
                Some(right),
                rules,
                2,
            )
            .unwrap();
            assert_eq!(moved.paperdoll.len(), 1);
            assert_eq!(*moved.paperdoll.get(&right).unwrap(), id(3));
            assert!(PaperdollMutation::equip(
                &third.inventory,
                &third.paperdoll,
                id(3),
                Some(EquipSlot::Rhand),
                rules,
                2
            )
            .is_err());
            let repeat =
                PaperdollMutation::equip(&third.inventory, &third.paperdoll, id(3), None, rules, 2)
                    .unwrap();
            assert!(repeat.changes.is_empty());
            assert_eq!(repeat.inventory, third.inventory);
        }
    }
    #[test]
    fn bow_installs_lowest_matching_ammo_and_shield_removes_weapon_and_charge_atomically() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let inventory = items(&[13, 1341, 17, 17, 18]);
        let bow =
            PaperdollMutation::equip(&inventory, &BTreeMap::new(), id(1), None, rules, 2).unwrap();
        assert_eq!(*bow.paperdoll.get(&EquipSlot::Rhand).unwrap(), id(1));
        assert_eq!(*bow.paperdoll.get(&EquipSlot::Lhand).unwrap(), id(3));
        assert_eq!(
            bow.changes
                .iter()
                .map(|c| c.after.as_ref().unwrap().id)
                .collect::<Vec<_>>(),
            vec![id(1), id(3)]
        );
        let mut charged = bow.inventory.clone();
        charged.get_mut(&id(1)).unwrap().charges = 7;
        let shield =
            PaperdollMutation::equip(&charged, &bow.paperdoll, id(5), None, rules, 2).unwrap();
        assert!(!shield.paperdoll.contains_key(&EquipSlot::Rhand));
        assert_eq!(*shield.paperdoll.get(&EquipSlot::Lhand).unwrap(), id(5));
        assert_eq!(shield.inventory.get(&id(1)).unwrap().charges, 0);
        assert_eq!(shield.inventory.get(&id(3)).unwrap().location, ItemLocation::Inventory);
        assert_eq!(charged.get(&id(1)).unwrap().charges, 7);
        let removed =
            PaperdollMutation::unequip(&charged, &bow.paperdoll, EquipSlot::Rhand, rules, 2)
                .unwrap();
        assert!(removed.paperdoll.is_empty());
        assert_eq!(removed.inventory.values().map(|i| i.count).sum::<u64>(), 5);
    }
    #[test]
    fn full_armor_excludes_legs_but_grade_does_not_block_equipping() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let inventory = items(&[28, 60]);
        let legs =
            PaperdollMutation::equip(&inventory, &BTreeMap::new(), id(1), None, rules, 2).unwrap();
        let full =
            PaperdollMutation::equip(&legs.inventory, &legs.paperdoll, id(2), None, rules, 2)
                .unwrap();
        assert_eq!(full.paperdoll, BTreeMap::from([(EquipSlot::Chest, id(2))]));
        let restored =
            PaperdollMutation::equip(&full.inventory, &full.paperdoll, id(1), None, rules, 2)
                .unwrap();
        assert_eq!(restored.paperdoll, BTreeMap::from([(EquipSlot::Legs, id(1))]));
        assert_eq!(
            inventory_totals(&inventory, rules).unwrap(),
            inventory_totals(&restored.inventory, rules).unwrap()
        );
        let mut corrupt = full.inventory.clone();
        let mut index = full.paperdoll.clone();
        corrupt.get_mut(&id(1)).unwrap().slot = Some(EquipSlot::Legs);
        corrupt.get_mut(&id(1)).unwrap().location = ItemLocation::Paperdoll;
        index.insert(EquipSlot::Legs, id(1));
        assert!(validate_paperdoll(&corrupt, &index, rules).is_err());
    }
}
