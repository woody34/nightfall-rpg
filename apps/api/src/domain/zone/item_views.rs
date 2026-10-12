//! Owner inventory projections and the deliberately smaller public paperdoll projection.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::phase4::ItemReuse;
use super::phase4_checkpoint::ItemReuseKey;
use super::{
    EntityId, EquipSlot, ItemError, ItemInstance, ItemInstanceId, ItemLocation, Phase4Player,
    Phase4Rules, Tick,
};

/// Owner-visible item fields. Provider history and creation ticks stay private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct OwnedItem {
    pub id: ItemInstanceId,
    pub template_id: u32,
    pub count: u64,
    pub enchant: u32,
    pub augment_id: Option<u32>,
    pub location: ItemLocation,
    pub slot: Option<EquipSlot>,
    pub charges: u8,
    pub revision: u64,
}
impl From<&ItemInstance> for OwnedItem {
    fn from(item: &ItemInstance) -> Self {
        Self {
            id: item.id,
            template_id: item.template_id,
            count: item.count,
            enchant: item.enchant,
            augment_id: item.augment_id,
            location: item.location,
            slot: item.slot,
            charges: item.charges,
            revision: item.revision,
        }
    }
}
impl OwnedItem {
    fn validate(&self) -> Result<(), ItemError> {
        ItemInstance {
            id: self.id,
            template_id: self.template_id,
            count: self.count,
            enchant: self.enchant,
            augment_id: self.augment_id,
            location: self.location,
            slot: self.slot,
            charges: self.charges,
            revision: self.revision,
            created_tick: Tick(0),
        }
        .validate()
    }
}

/// Public art policy I6; template zero is a changed-slot tombstone only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct VisibleEquipment {
    pub slot: EquipSlot,
    pub template_id: u32,
    pub glow_tier: u8,
}

/// Raw carried-weight display; the separate bonus participates in penalty calculation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemWeightView {
    pub weight: i64,
    pub max_load: i64,
    pub penalty_bonus: i64,
    pub per_mille: i64,
}
impl ItemWeightView {
    /// Match the detached inventory display using checked wide arithmetic.
    pub fn new(weight: i64, max_load: i64, penalty_bonus: i64) -> Result<Self, ItemError> {
        if weight < 0 || max_load <= 0 {
            return Err(ItemError("invalid displayed item weight"));
        }
        let per_mille = i128::from(weight)
            .checked_mul(1000)
            .and_then(|v| v.checked_div(i128::from(max_load)))
            .and_then(|v| i64::try_from(v).ok())
            .ok_or(ItemError("item weight display overflow"))?;
        Ok(Self {
            weight,
            max_load,
            penalty_bonus,
            per_mille,
        })
    }
    fn validate(&self) -> Result<(), ItemError> {
        if &Self::new(self.weight, self.max_load, self.penalty_bonus)? != self {
            return Err(ItemError("inconsistent displayed item weight"));
        }
        Ok(())
    }
}

/// Authoritative folded penalty scalars, including the historical movement flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemPenaltyView {
    pub expertise: u8,
    pub weapon_level: u8,
    pub armor_level: u8,
    pub overload_level: u8,
    pub overloaded: bool,
}
impl From<&Phase4Player> for ItemPenaltyView {
    fn from(player: &Phase4Player) -> Self {
        Self {
            expertise: player.expertise,
            weapon_level: player.weapon_penalty,
            armor_level: player.armor_penalty,
            overload_level: player.overload_level,
            overloaded: player.overloaded,
        }
    }
}
impl ItemPenaltyView {
    fn validate(&self) -> Result<(), ItemError> {
        if self.expertise > 7
            || self.weapon_level > 4
            || self.armor_level > 4
            || self.overload_level > 4
        {
            return Err(ItemError("invalid displayed item penalties"));
        }
        Ok(())
    }
}

/// Full owner replacement. Ordered vectors retain duplicate detection at replay boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct InventoryView {
    pub inventory_revision: u64,
    pub items: Vec<OwnedItem>,
    pub paperdoll: Vec<(EquipSlot, ItemInstanceId)>,
    pub auto_shots: Vec<u32>,
    pub reuse: Vec<ItemReuse>,
    pub weight: ItemWeightView,
    pub penalties: ItemPenaltyView,
    pub rules_hash: [u8; 32],
    pub ordinary_capacity: u32,
    pub quest_capacity: u32,
}
impl InventoryView {
    /// Read final authoritative scalars without recalculating providers or paying reuse.
    pub fn from_player(player: &Phase4Player, rules_hash: [u8; 32]) -> Result<Self, ItemError> {
        let view = Self {
            inventory_revision: player.inventory_revision,
            items: player.inventory.values().map(OwnedItem::from).collect(),
            paperdoll: player
                .paperdoll
                .iter()
                .map(|(slot, id)| (*slot, *id))
                .collect(),
            auto_shots: player.auto_shots.iter().copied().collect(),
            reuse: player.item_reuse.clone(),
            weight: ItemWeightView::new(
                player.weight,
                player.max_load,
                player.weight_penalty_bonus,
            )?,
            penalties: player.into(),
            rules_hash,
            ordinary_capacity: player.ordinary_capacity,
            quest_capacity: player.quest_capacity,
        };
        view.validate()?;
        Ok(view)
    }
    /// Structural DTO checks; immutable rules already validate authoritative inventory.
    pub fn validate(&self) -> Result<(), ItemError> {
        check_revision(self.inventory_revision)?;
        validate_items(&self.items, self.inventory_revision)?;
        validate_reuse(&self.auto_shots, &self.reuse)?;
        self.weight.validate()?;
        self.penalties.validate()?;
        let mut expected = BTreeMap::new();
        for item in &self.items {
            if let Some(slot) = item.slot {
                if expected.insert(slot, item.id).is_some() {
                    return Err(ItemError("duplicate displayed equipment slot"));
                }
            }
        }
        if self.paperdoll != expected.into_iter().collect::<Vec<_>>()
            || self.ordinary_capacity > i32::MAX as u32
            || self.quest_capacity > i32::MAX as u32
        {
            return Err(ItemError("invalid displayed paperdoll or capacity"));
        }
        Ok(())
    }
}

/// Canonical live output for replay tags32–36. Only `EquipmentChanged` is public.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemViewEvent {
    pub entity: EntityId,
    pub incarnation: u32,
    pub tick: Tick,
    pub detail: ItemViewDetail,
}
/// Owner revision is deliberately absent from the public equipment variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum ItemViewDetail {
    InventorySync {
        baseline_tick: Tick,
        inventory: InventoryView,
    },
    InventoryChanged {
        previous_revision: u64,
        inventory_revision: u64,
        added: Vec<OwnedItem>,
        removed: Vec<ItemInstanceId>,
        updated: Vec<OwnedItem>,
        auto_shots: Vec<u32>,
        reuse: Vec<ItemReuse>,
    },
    EquipmentChanged {
        visible: Vec<VisibleEquipment>,
    },
    WeightChanged {
        inventory_revision: u64,
        weight: ItemWeightView,
    },
    PenaltyChanged {
        inventory_revision: u64,
        penalties: ItemPenaltyView,
    },
}
impl ItemViewEvent {
    /// Public equipment is indexed by the owner's cell; all other variants are owner-only.
    pub const fn owner_only(&self) -> bool {
        !matches!(self.detail, ItemViewDetail::EquipmentChanged { .. })
    }
    /// Reject malformed, reordered, duplicate and overlapping item records before replay.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.entity.as_uuid().is_nil() || self.incarnation == 0 || self.tick.0 > i64::MAX as u64
        {
            return Err(ItemError("invalid live item projection identity"));
        }
        match &self.detail {
            ItemViewDetail::InventorySync {
                baseline_tick,
                inventory,
            } => {
                if *baseline_tick != self.tick {
                    return Err(ItemError("invalid inventory baseline tick"));
                }
                inventory.validate()?;
            },
            ItemViewDetail::InventoryChanged {
                previous_revision,
                inventory_revision,
                added,
                removed,
                updated,
                auto_shots,
                reuse,
            } => {
                check_revision(*inventory_revision)?;
                if previous_revision.checked_add(1) != Some(*inventory_revision) {
                    return Err(ItemError("nonconsecutive inventory delta"));
                }
                validate_items(added, *inventory_revision)?;
                validate_items(updated, *inventory_revision)?;
                if added
                    .iter()
                    .chain(updated)
                    .any(|item| item.revision != *inventory_revision)
                {
                    return Err(ItemError("changed item does not carry the final revision"));
                }
                validate_reuse(auto_shots, reuse)?;
                if !strictly_ordered(removed.iter().copied()) {
                    return Err(ItemError("unordered removed items"));
                }
                let all: Vec<_> = added
                    .iter()
                    .chain(updated)
                    .map(|item| item.id)
                    .chain(removed.iter().copied())
                    .collect();
                if all.iter().copied().collect::<BTreeSet<_>>().len() != all.len() {
                    return Err(ItemError("overlapping item delta"));
                }
            },
            ItemViewDetail::EquipmentChanged { visible } => validate_visible(visible, true)?,
            ItemViewDetail::WeightChanged {
                inventory_revision,
                weight,
            } => {
                check_revision(*inventory_revision)?;
                weight.validate()?;
            },
            ItemViewDetail::PenaltyChanged {
                inventory_revision,
                penalties,
            } => {
                check_revision(*inventory_revision)?;
                penalties.validate()?;
            },
        }
        Ok(())
    }
}
fn check_revision(revision: u64) -> Result<(), ItemError> {
    if revision > i64::MAX as u64 {
        return Err(ItemError("displayed revision outside bigint"));
    }
    Ok(())
}
fn strictly_ordered<T: Ord>(values: impl Iterator<Item = T>) -> bool {
    let mut previous = None;
    for value in values {
        if previous.as_ref().is_some_and(|old| old >= &value) {
            return false;
        }
        previous = Some(value);
    }
    true
}
fn validate_items(items: &[OwnedItem], revision: u64) -> Result<(), ItemError> {
    if !strictly_ordered(items.iter().map(|item| item.id)) {
        return Err(ItemError("unordered displayed items"));
    }
    for item in items {
        item.validate()?;
        if item.revision > revision {
            return Err(ItemError("future displayed item revision"));
        }
    }
    Ok(())
}
fn validate_reuse(auto_shots: &[u32], reuse: &[ItemReuse]) -> Result<(), ItemError> {
    if !strictly_ordered(auto_shots.iter().copied())
        || auto_shots
            .iter()
            .any(|id| *id == 0 || *id > i32::MAX as u32)
        || !strictly_ordered(reuse.iter().map(|entry| entry.key))
    {
        return Err(ItemError("unordered item reuse or auto shots"));
    }
    for entry in reuse {
        let (ItemReuseKey::Template(key) | ItemReuseKey::SharedGroup(key)) = entry.key;
        if key == 0 || key > i32::MAX as u32 || entry.until.0 > i64::MAX as u64 {
            return Err(ItemError("invalid displayed reuse"));
        }
    }
    Ok(())
}
/// Shared validation for full spawn equipment and changed-slot tombstones.
pub fn validate_visible(visible: &[VisibleEquipment], delta: bool) -> Result<(), ItemError> {
    if (delta && visible.is_empty()) || !strictly_ordered(visible.iter().map(|entry| entry.slot)) {
        return Err(ItemError("unordered visible equipment"));
    }
    for entry in visible {
        if entry.template_id > i32::MAX as u32
            || entry.glow_tier > 2
            || (entry.template_id == 0 && (!delta || entry.glow_tier != 0))
        {
            return Err(ItemError("invalid visible equipment"));
        }
    }
    Ok(())
}
/// Presentation-only milestones from Phase4 design §1.5/I6; exact enchant stays private.
pub fn visible_equipment(
    player: &Phase4Player,
    rules: &Phase4Rules,
) -> Result<Vec<VisibleEquipment>, ItemError> {
    player
        .paperdoll
        .iter()
        .map(|(slot, id)| {
            let item = player
                .inventory
                .get(id)
                .ok_or(ItemError("missing visible equipment instance"))?;
            let definition = rules
                .parts()
                .items
                .get(&item.template_id)
                .ok_or(ItemError("missing visible equipment template"))?;
            let glow_tier = match definition.kind.as_str() {
                "Weapon" if item.enchant >= 15 => 2,
                "Weapon" if item.enchant >= 7 => 1,
                "Armor" if item.enchant >= 6 => 1,
                _ => 0,
            };
            Ok(VisibleEquipment {
                slot: *slot,
                template_id: item.template_id,
                glow_tier,
            })
        })
        .collect()
}

/// Compare two committed owner projections; round-trip equip still reports item revisions.
pub fn inventory_delta(
    before: &InventoryView,
    after: &InventoryView,
) -> Result<Option<ItemViewDetail>, ItemError> {
    before.validate()?;
    after.validate()?;
    if before.inventory_revision == after.inventory_revision {
        if before.items != after.items
            || before.paperdoll != after.paperdoll
            || before.auto_shots != after.auto_shots
            || before.reuse != after.reuse
        {
            return Err(ItemError("inventory changed without a revision"));
        }
        return Ok(None);
    }
    if before.inventory_revision.checked_add(1) != Some(after.inventory_revision) {
        return Err(ItemError("inventory delta skipped a revision"));
    }
    let old: BTreeMap<_, _> = before.items.iter().map(|item| (item.id, item)).collect();
    let new: BTreeMap<_, _> = after.items.iter().map(|item| (item.id, item)).collect();
    Ok(Some(ItemViewDetail::InventoryChanged {
        previous_revision: before.inventory_revision,
        inventory_revision: after.inventory_revision,
        added: new
            .values()
            .filter(|item| !old.contains_key(&item.id))
            .map(|item| (*item).clone())
            .collect(),
        removed: old
            .keys()
            .filter(|id| !new.contains_key(id))
            .copied()
            .collect(),
        updated: new
            .values()
            .filter(|item| old.get(&item.id).is_some_and(|prior| prior != *item))
            .map(|item| (*item).clone())
            .collect(),
        auto_shots: after.auto_shots.clone(),
        reuse: after.reuse.clone(),
    }))
}
/// Only changed public slots; removing the last item still produces its explicit tombstone.
pub fn equipment_delta(
    before: &[VisibleEquipment],
    after: &[VisibleEquipment],
) -> Result<Vec<VisibleEquipment>, ItemError> {
    validate_visible(before, false)?;
    validate_visible(after, false)?;
    let old: BTreeMap<_, _> = before.iter().map(|entry| (entry.slot, entry)).collect();
    let new: BTreeMap<_, _> = after.iter().map(|entry| (entry.slot, entry)).collect();
    let slots: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    Ok(slots
        .into_iter()
        .filter(|slot| old.get(slot) != new.get(slot))
        .map(|slot| {
            new.get(&slot).map_or(
                VisibleEquipment {
                    slot,
                    template_id: 0,
                    glow_tier: 0,
                },
                |entry| (*entry).clone(),
            )
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn inventory() -> InventoryView {
        InventoryView {
            inventory_revision: 1,
            items: vec![OwnedItem {
                id: ItemInstanceId::new(1).unwrap(),
                template_id: 17,
                count: 10,
                enchant: 0,
                augment_id: None,
                location: ItemLocation::Inventory,
                slot: None,
                charges: 0,
                revision: 1,
            }],
            paperdoll: Vec::new(),
            auto_shots: Vec::new(),
            reuse: Vec::new(),
            weight: ItemWeightView::new(10, 1000, 5).unwrap(),
            penalties: ItemPenaltyView {
                expertise: 0,
                weapon_level: 0,
                armor_level: 0,
                overload_level: 0,
                overloaded: false,
            },
            rules_hash: [4; 32],
            ordinary_capacity: 80,
            quest_capacity: 100,
        }
    }
    #[test]
    fn committed_deltas_preserve_item_identity_and_public_tombstones() {
        let before = inventory();
        let mut after = before.clone();
        after.inventory_revision = 2;
        after.items[0].revision = 2;
        let ItemViewDetail::InventoryChanged {
            added,
            removed,
            updated,
            ..
        } = inventory_delta(&before, &after).unwrap().unwrap()
        else {
            panic!("delta");
        };
        assert!(added.is_empty() && removed.is_empty());
        assert_eq!(updated, after.items);
        after.items[0].id = ItemInstanceId::new(2).unwrap();
        let detail = inventory_delta(&before, &after).unwrap().unwrap();
        let event = ItemViewEvent {
            entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
            incarnation: 1,
            tick: Tick(5),
            detail,
        };
        event.validate().unwrap();
        let ItemViewDetail::InventoryChanged {
            added,
            removed,
            updated,
            ..
        } = event.detail
        else {
            panic!("delta");
        };
        assert_eq!(added, after.items);
        assert_eq!(removed, vec![before.items[0].id]);
        assert!(updated.is_empty());
        let gear = vec![VisibleEquipment {
            slot: EquipSlot::Rhand,
            template_id: 1,
            glow_tier: 1,
        }];
        assert!(equipment_delta(&gear, &gear).unwrap().is_empty());
        assert_eq!(
            equipment_delta(&gear, &[]).unwrap(),
            vec![VisibleEquipment {
                slot: EquipSlot::Rhand,
                template_id: 0,
                glow_tier: 0
            }]
        );
        assert!(validate_visible(&equipment_delta(&gear, &[]).unwrap(), false).is_err());
    }
    #[test]
    fn malformed_inventory_views_fail_before_publication() {
        let base = inventory();
        base.validate().unwrap();
        for change in 0..6 {
            let mut bad = base.clone();
            match change {
                0 => bad.items.push(bad.items[0].clone()),
                1 => bad.paperdoll.push((EquipSlot::Rhand, bad.items[0].id)),
                2 => bad.items[0].revision = 2,
                3 => bad.weight.per_mille = 11,
                4 => bad.auto_shots = vec![1, 1],
                _ => bad.reuse.push(ItemReuse {
                    key: ItemReuseKey::Template(0),
                    until: Tick(7),
                }),
            }
            assert!(bad.validate().is_err(), "case {change}");
        }
        let mut after = base.clone();
        after.inventory_revision = 3;
        assert!(inventory_delta(&base, &after).is_err());
        assert!(ItemWeightView::new(i64::MAX, 1, 0).is_err());
        assert_eq!(
            ItemWeightView::new(i64::MAX, i64::MAX, 0)
                .unwrap()
                .per_mille,
            1000
        );
    }
}
