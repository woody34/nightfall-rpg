//! Paused item persistence; no command callbacks, timed effects or on-start hooks.
//! Equipment provider order and set-shield/overload history survive death (D16/D20).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::phase4::ItemReuse;
use super::{
    EntityId, EquipSlot, ItemError, ItemInstance, ItemInstanceId, ItemLocation, Phase3Snapshot,
    Phase4Player, Phase4Snapshot, RestoredModifierSource, RestoredPhase3Timers, Tick,
};

/// Retained items preserve exact identity, charge, enchant and augmentation scalars.
pub type RetainedItem = ItemInstance;

/// Shared reuse survives consumption of the clicked instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ItemReuseKey {
    /// No positive shared group: key by template.
    Template(u32),
    /// Positive source shared reuse group.
    SharedGroup(u32),
}

/// One paused reuse entry. A list permits a strict duplicate/order check during restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RetainedItemReuse {
    pub key: ItemReuseKey,
    pub remaining_ticks: u64,
}

/// Stable provider ownership, independent of ordinary `ActiveEffect` lifecycle flags.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum EquipmentSourceKey {
    Item {
        instance: ItemInstanceId,
    },
    ItemSkill {
        instance: ItemInstanceId,
        xml_ordinal: u32,
    },
    Enchant4 {
        instance: ItemInstanceId,
    },
    EnchantOption {
        instance: ItemInstanceId,
        option: u32,
    },
    Augment {
        instance: ItemInstanceId,
        half: u8,
    },
    ArmorSet {
        chest: ItemInstanceId,
        set: u32,
        skill_kind: SetSkillKind,
        ordinal: u32,
    },
    GradePenalty {
        weapon: bool,
    },
    Overload,
    Polearm,
}

/// Armour-set providers have independent lifetime and source lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetSkillKind {
    /// All required pieces present.
    Full,
    /// Persisted listener activation; does not imply a full set.
    Shield,
    /// Every required piece enchanted to six.
    Enchant6,
}

/// D20: shield activation is historical and must never be inferred from full-set state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SetActivation {
    pub chest: ItemInstanceId,
    pub full_set_active: bool,
    pub shield_skill_active: bool,
    pub enchant6_active: bool,
}

/// Cross-source retained order. Phase3 indexes reference its canonical retained order,
/// rather than duplicating skill/effect state or converting equipment into timed effects.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RetainedEquipmentSource {
    /// Position in `Phase3Checkpoint.modifier_install_order`.
    Phase3(u32),
    /// Equipment provider, reallocated once in this relative position on admission.
    Equipment(EquipmentSourceKey),
}

/// Materialization proof. Absence means a pending first kit, never an implicit empty kit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct StarterKitReceipt {
    pub manifest_version: u32,
    pub manifest_hash: [u8; 32],
    pub source_class: crate::domain::class::ClassId,
    pub request_id: uuid::Uuid,
    pub materialized_revision: u64,
}

/// Complete offline item payload (design §9.1). Snapshot recovery uses absolute timers;
/// fresh character admission uses these paused remainders. No pending action is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase4Checkpoint {
    pub schema_version: u32,
    pub timer_policy_version: u32,
    pub rules_hash: [u8; 32],
    pub inventory_revision: u64,
    pub instances: Vec<RetainedItem>,
    pub paperdoll: BTreeMap<EquipSlot, ItemInstanceId>,
    pub auto_shots: BTreeSet<u32>,
    pub item_reuse_remainders: Vec<RetainedItemReuse>,
    pub equipment_install_order: Vec<RetainedEquipmentSource>,
    pub active_sets: BTreeMap<u32, SetActivation>,
    pub weight: i64,
    pub max_load: i64,
    pub ordinary_capacity: u32,
    pub quest_capacity: u32,
    pub weight_penalty_bonus: i64,
    pub overload_level: u8,
    pub overloaded: bool,
    pub expertise: u8,
    pub weapon_penalty: u8,
    pub armor_penalty: u8,
    pub starter_kit_receipt: Option<StarterKitReceipt>,
}

impl Phase4Checkpoint {
    /// Validate structural persistence invariants without pretending to validate the
    /// catalogue-dependent fold. Admission additionally checks against immutable rules.
    pub fn validate_structure(&self, phase3_sources: usize) -> Result<(), ItemError> {
        if self.schema_version != 1 || self.timer_policy_version != 1 {
            return Err(ItemError("unsupported Phase4 checkpoint policy"));
        }
        if self.inventory_revision > i64::MAX as u64
            || self.ordinary_capacity > i32::MAX as u32
            || self.quest_capacity > i32::MAX as u32
            || self.weight < 0
            || self.overload_level > 4
            || self.weapon_penalty > 4
            || self.armor_penalty > 4
            || self.expertise > 7
        {
            return Err(ItemError("invalid inventory revision or penalty scalar"));
        }
        let mut previous = None;
        let mut equipped = BTreeMap::new();
        for item in &self.instances {
            item.validate()?;
            if previous.is_some_and(|id| id >= item.id) || item.revision > self.inventory_revision {
                return Err(ItemError("unordered item IDs or future revision"));
            }
            previous = Some(item.id);
            if let Some(slot) = item.slot {
                if equipped.insert(slot, item.id).is_some() {
                    return Err(ItemError("duplicate paperdoll slot"));
                }
            }
        }
        if equipped != self.paperdoll {
            return Err(ItemError("paperdoll does not match inventory"));
        }
        if self.auto_shots.contains(&0) {
            return Err(ItemError("invalid auto-shot template"));
        }
        self.validate_reuse()?;
        let mut sources = BTreeSet::new();
        let mut retained_phase3 = BTreeSet::new();
        for source in &self.equipment_install_order {
            if !sources.insert(source) {
                return Err(ItemError("duplicate retained provider"));
            }
            match source {
                RetainedEquipmentSource::Phase3(index) => {
                    let index = usize::try_from(*index)
                        .map_err(|_| ItemError("provider index overflow"))?;
                    if index >= phase3_sources || index != retained_phase3.len() {
                        return Err(ItemError("invalid Phase3 provider index"));
                    }
                    retained_phase3.insert(index);
                },
                RetainedEquipmentSource::Equipment(key) => self.validate_provider(key)?,
            }
        }
        if retained_phase3.len() != phase3_sources {
            return Err(ItemError("missing retained Phase3 provider"));
        }
        for (set, activation) in &self.active_sets {
            if *set == 0
                || self.paperdoll.get(&EquipSlot::Chest) != Some(&activation.chest)
                || (activation.enchant6_active && !activation.full_set_active)
            {
                return Err(ItemError("invalid retained armour set"));
            }
        }
        if let Some(receipt) = &self.starter_kit_receipt {
            if receipt.manifest_version != 1
                || receipt.request_id.is_nil()
                || receipt.materialized_revision > self.inventory_revision
            {
                return Err(ItemError("invalid starter materialization receipt"));
            }
        }
        Ok(())
    }

    fn validate_provider(&self, key: &EquipmentSourceKey) -> Result<(), ItemError> {
        let instance = match key {
            EquipmentSourceKey::Item { instance }
            | EquipmentSourceKey::ItemSkill { instance, .. }
            | EquipmentSourceKey::Enchant4 { instance }
            | EquipmentSourceKey::EnchantOption { instance, .. } => Some(*instance),
            EquipmentSourceKey::Augment { instance, half } => {
                if *half > 1 {
                    return Err(ItemError("invalid augmentation half"));
                }
                Some(*instance)
            },
            EquipmentSourceKey::ArmorSet {
                chest,
                set,
                skill_kind,
                ..
            } => {
                let active = self
                    .active_sets
                    .get(set)
                    .filter(|active| active.chest == *chest)
                    .ok_or(ItemError("orphan armour-set provider"))?;
                let enabled = match skill_kind {
                    SetSkillKind::Full => active.full_set_active,
                    SetSkillKind::Shield => active.shield_skill_active,
                    SetSkillKind::Enchant6 => active.enchant6_active,
                };
                if !enabled {
                    return Err(ItemError("inactive armour-set provider"));
                }
                Some(*chest)
            },
            EquipmentSourceKey::GradePenalty { weapon } => {
                if (if *weapon {
                    self.weapon_penalty
                } else {
                    self.armor_penalty
                }) == 0
                {
                    return Err(ItemError("inactive grade provider"));
                }
                None
            },
            EquipmentSourceKey::Overload => {
                if self.overload_level == 0 {
                    return Err(ItemError("inactive overload provider"));
                }
                None
            },
            EquipmentSourceKey::Polearm => None,
        };
        if let Some(id) = instance {
            let item = self
                .instances
                .iter()
                .find(|item| item.id == id)
                .ok_or(ItemError("orphan item provider"))?;
            if item.location != ItemLocation::Paperdoll {
                return Err(ItemError("unequipped item provider"));
            }
        }
        Ok(())
    }

    fn validate_reuse(&self) -> Result<(), ItemError> {
        let mut previous = None;
        for reuse in &self.item_reuse_remainders {
            if reuse.remaining_ticks == 0
                || previous.is_some_and(|key| key >= reuse.key)
                || matches!(reuse.key, ItemReuseKey::Template(0) | ItemReuseKey::SharedGroup(0))
            {
                return Err(ItemError("invalid item reuse remainder"));
            }
            previous = Some(reuse.key);
        }
        Ok(())
    }

    /// Rebase every timer as a single checked plan; elapsed offline time is irrelevant.
    pub fn restore_reuse(
        &self,
        admission: Tick,
    ) -> Result<BTreeMap<ItemReuseKey, Tick>, ItemError> {
        self.validate_reuse()?;
        self.item_reuse_remainders
            .iter()
            .map(|reuse| {
                admission
                    .0
                    .checked_add(reuse.remaining_ticks)
                    .map(|until| (reuse.key, Tick(until)))
                    .ok_or(ItemError("item reuse deadline overflow"))
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn retained() -> Phase4Checkpoint {
        let id = ItemInstanceId::new(1).unwrap();
        Phase4Checkpoint {
            schema_version: 1,
            timer_policy_version: 1,
            rules_hash: [1; 32],
            inventory_revision: 1,
            instances: vec![ItemInstance {
                creation_origin: None,
                id,
                template_id: 1146,
                count: 1,
                location: ItemLocation::Paperdoll,
                slot: Some(EquipSlot::Chest),
                enchant: u32::MAX,
                augment_id: Some(u32::MAX),
                charges: 0,
                created_tick: Tick(0),
                revision: 1,
            }],
            paperdoll: BTreeMap::from([(EquipSlot::Chest, id)]),
            auto_shots: BTreeSet::new(),
            item_reuse_remainders: vec![RetainedItemReuse {
                key: ItemReuseKey::Template(736),
                remaining_ticks: 10,
            }],
            equipment_install_order: vec![RetainedEquipmentSource::Equipment(
                EquipmentSourceKey::ArmorSet {
                    chest: id,
                    set: 1,
                    skill_kind: SetSkillKind::Shield,
                    ordinal: 0,
                },
            )],
            active_sets: BTreeMap::from([(
                1,
                SetActivation {
                    chest: id,
                    full_set_active: false,
                    shield_skill_active: true,
                    enchant6_active: false,
                },
            )]),
            weight: 1000,
            max_load: 1000,
            ordinary_capacity: 80,
            quest_capacity: 100,
            weight_penalty_bonus: 1,
            overload_level: 4,
            overloaded: true,
            expertise: 0,
            weapon_penalty: 0,
            armor_penalty: 0,
            starter_kit_receipt: None,
        }
    }

    #[test]
    fn retained_history_and_u32_options_roundtrip_without_recomputing_flags() {
        let state = retained();
        state.validate_structure(0).unwrap();
        let bytes = serde_json::to_vec(&state).unwrap();
        let restored: Phase4Checkpoint = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, state);
        assert_eq!(
            restored
                .restore_reuse(Tick(50))
                .unwrap()
                .get(&ItemReuseKey::Template(736)),
            Some(&Tick(60))
        );
        assert!(restored.restore_reuse(Tick(u64::MAX)).is_err());
    }

    #[test]
    fn corruption_never_normalizes_inventory_or_provider_order() {
        let state = retained();
        let mut bad = state.clone();
        bad.instances.push(bad.instances.first().unwrap().clone());
        assert!(bad.validate_structure(0).is_err());
        let mut bad = state.clone();
        bad.paperdoll.clear();
        assert!(bad.validate_structure(0).is_err());
        let mut bad = state.clone();
        bad.active_sets.get_mut(&1).unwrap().shield_skill_active = false;
        assert!(bad.validate_structure(0).is_err());
        let mut bad = state.clone();
        bad.equipment_install_order
            .push(bad.equipment_install_order.first().unwrap().clone());
        assert!(bad.validate_structure(0).is_err());
        assert!(state.validate_structure(1).is_err());
        let mut valid = state;
        valid
            .equipment_install_order
            .push(RetainedEquipmentSource::Phase3(0));
        valid.validate_structure(1).unwrap();
    }
}

impl Phase4Snapshot {
    /// Freeze item state at the same boundary as the Phase3 checkpoint. Timed actions are
    /// absent offline; retained equipment providers interleave with retained skill sources.
    pub fn checkpoint(
        &self,
        entity: EntityId,
        tick: Tick,
        skills: &Phase3Snapshot,
    ) -> Result<Phase4Checkpoint, ItemError> {
        let player = self
            .players
            .get(&entity)
            .ok_or(ItemError("missing item owner"))?;
        let side = skills
            .entities
            .get(&entity)
            .ok_or(ItemError("missing skill owner"))?;
        let phase3 = skills
            .checkpoint(entity, tick)
            .map_err(|_| ItemError("invalid retained skill state"))?;
        let mut order = BTreeMap::new();
        let mut index = 0_u32;
        for (sequence, source) in &side.modifier_install_order {
            if let RestoredModifierSource::Effect(id) = source {
                let effect = side
                    .effects
                    .entries()
                    .get(id)
                    .ok_or(ItemError("orphan skill provider"))?;
                if effect
                    .retain(tick)
                    .map_err(|_| ItemError("invalid retained effect"))?
                    .is_none()
                {
                    continue;
                }
            }
            order.insert(*sequence, RetainedEquipmentSource::Phase3(index));
            index = index
                .checked_add(1)
                .ok_or(ItemError("provider index overflow"))?;
        }
        if usize::try_from(index).ok() != Some(phase3.modifier_install_order.len()) {
            return Err(ItemError("retained skill order mismatch"));
        }
        for (sequence, source) in &player.equipment_sources {
            if order
                .insert(*sequence, RetainedEquipmentSource::Equipment(source.clone()))
                .is_some()
            {
                return Err(ItemError("duplicate shared provider sequence"));
            }
        }
        let checkpoint = Phase4Checkpoint {
            schema_version: 1,
            timer_policy_version: self.timer_policy_version,
            rules_hash: self.rules_hash,
            inventory_revision: player.inventory_revision,
            instances: player.inventory.values().cloned().collect(),
            paperdoll: player.paperdoll.clone(),
            auto_shots: player.auto_shots.clone(),
            item_reuse_remainders: player
                .item_reuse
                .iter()
                .filter_map(|entry| {
                    entry
                        .until
                        .0
                        .checked_sub(tick.0)
                        .filter(|left| *left > 0)
                        .map(|remaining_ticks| RetainedItemReuse {
                            key: entry.key,
                            remaining_ticks,
                        })
                })
                .collect(),
            equipment_install_order: order.into_values().collect(),
            active_sets: player.active_sets.clone(),
            weight: player.weight,
            max_load: player.max_load,
            ordinary_capacity: player.ordinary_capacity,
            quest_capacity: player.quest_capacity,
            weight_penalty_bonus: player.weight_penalty_bonus,
            overload_level: player.overload_level,
            overloaded: player.overloaded,
            expertise: player.expertise,
            weapon_penalty: player.weapon_penalty,
            armor_penalty: player.armor_penalty,
            starter_kit_receipt: Some(player.starter_kit_receipt.clone()),
        };
        checkpoint.validate_structure(phase3.modifier_install_order.len())?;
        Ok(checkpoint)
    }
}
impl Phase4Player {
    /// Combine staged skill restoration and equipment into ONE new shared sequence range.
    /// The caller supplies the original allocation cursor, before staged Phase3 restoration.
    /// Neither a rejected plan nor cold reconstruction publishes intermediate sequences.
    pub fn restore(
        checkpoint: &Phase4Checkpoint,
        timers: &RestoredPhase3Timers,
        admission: Tick,
        first_sequence: u64,
    ) -> Result<(Self, RestoredPhase3Timers), ItemError> {
        let phase3_sources: Vec<_> = timers.modifier_install_order.values().cloned().collect();
        checkpoint.validate_structure(phase3_sources.len())?;
        let receipt = checkpoint
            .starter_kit_receipt
            .clone()
            .ok_or(ItemError("unmaterialized starter kit"))?;
        let mut expected = first_sequence;
        for sequence in timers.modifier_install_order.keys() {
            if *sequence != expected {
                return Err(ItemError("staged Phase3 allocation mismatch"));
            }
            expected = expected
                .checked_add(1)
                .ok_or(ItemError("provider sequence overflow"))?;
        }
        if first_sequence == 0 || timers.next_install_sequence != expected {
            return Err(ItemError("staged Phase3 allocation mismatch"));
        }
        let mut rebuilt = timers.clone();
        rebuilt.modifier_install_order.clear();
        let mut equipment_sources = BTreeMap::new();
        let mut next = first_sequence;
        for source in &checkpoint.equipment_install_order {
            let following = next
                .checked_add(1)
                .ok_or(ItemError("provider sequence overflow"))?;
            match source {
                RetainedEquipmentSource::Phase3(index) => {
                    let source = phase3_sources
                        .get(
                            usize::try_from(*index)
                                .map_err(|_| ItemError("provider index overflow"))?,
                        )
                        .ok_or(ItemError("unknown retained skill provider"))?;
                    rebuilt.modifier_install_order.insert(next, source.clone());
                },
                RetainedEquipmentSource::Equipment(source) => {
                    equipment_sources.insert(next, source.clone());
                },
            }
            next = following;
        }
        rebuilt.next_install_sequence = next;
        let item_reuse = checkpoint
            .restore_reuse(admission)?
            .into_iter()
            .map(|(key, until)| ItemReuse { key, until })
            .collect();
        // D71: a new incarnation keeps every retained canonical field except the transient
        // shot charges and auto-shot toggles (in-memory only in the pinned source).
        Ok((
            Self {
                inventory: checkpoint
                    .instances
                    .iter()
                    .map(|item| {
                        let mut item = item.clone();
                        item.charges = 0;
                        (item.id, item)
                    })
                    .collect(),
                paperdoll: checkpoint.paperdoll.clone(),
                auto_shots: BTreeSet::new(),
                item_reuse,
                equipment_sources,
                active_sets: checkpoint.active_sets.clone(),
                weight: checkpoint.weight,
                max_load: checkpoint.max_load,
                ordinary_capacity: checkpoint.ordinary_capacity,
                quest_capacity: checkpoint.quest_capacity,
                weight_penalty_bonus: checkpoint.weight_penalty_bonus,
                overload_level: checkpoint.overload_level,
                overloaded: checkpoint.overloaded,
                expertise: checkpoint.expertise,
                weapon_penalty: checkpoint.weapon_penalty,
                armor_penalty: checkpoint.armor_penalty,
                starter_kit_receipt: receipt,
                inventory_revision: checkpoint.inventory_revision,
            },
            rebuilt,
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod player_tests {
    use super::super::effects::EffectList;
    use super::super::phase4_checkpoint::{EquipmentSourceKey, ItemReuseKey, StarterKitReceipt};
    use super::super::skill::{SkillId, SkillRef};
    use super::*;
    use std::collections::BTreeSet;

    // Ordering fixture only: catalogue/fold eligibility is covered by live admission tests.
    fn checkpoint() -> Phase4Checkpoint {
        Phase4Checkpoint {
            schema_version: 1,
            timer_policy_version: 1,
            rules_hash: [1; 32],
            inventory_revision: 1,
            instances: Vec::new(),
            paperdoll: BTreeMap::new(),
            auto_shots: BTreeSet::new(),
            item_reuse_remainders: vec![RetainedItemReuse {
                key: ItemReuseKey::SharedGroup(9),
                remaining_ticks: 7,
            }],
            equipment_install_order: vec![
                RetainedEquipmentSource::Equipment(EquipmentSourceKey::Overload),
                RetainedEquipmentSource::Phase3(0),
                RetainedEquipmentSource::Equipment(EquipmentSourceKey::GradePenalty {
                    weapon: true,
                }),
                RetainedEquipmentSource::Phase3(1),
            ],
            active_sets: BTreeMap::new(),
            weight: 0,
            max_load: 1000,
            ordinary_capacity: 80,
            quest_capacity: 100,
            weight_penalty_bonus: 1,
            overload_level: 1,
            overloaded: true,
            expertise: 0,
            weapon_penalty: 1,
            armor_penalty: 0,
            starter_kit_receipt: Some(StarterKitReceipt {
                manifest_version: 1,
                manifest_hash: [2; 32],
                source_class: crate::domain::class::ClassId(0),
                request_id: uuid::Uuid::from_u128(1),
                materialized_revision: 1,
            }),
        }
    }
    fn timers(first: u64) -> RestoredPhase3Timers {
        RestoredPhase3Timers {
            flag_until: None,
            reuse: BTreeMap::new(),
            effects: EffectList::default(),
            modifier_install_order: BTreeMap::from([
                (
                    first,
                    RestoredModifierSource::Passive(SkillRef {
                        id: SkillId(239),
                        rank: 1,
                    }),
                ),
                (first.checked_add(1).unwrap(), RestoredModifierSource::DeathPenalty),
            ]),
            next_effect_id: 1,
            next_install_sequence: first.checked_add(2).unwrap(),
        }
    }
    #[test]
    fn restoration_interleaves_once_and_preserves_history_and_paused_reuse() {
        let old = timers(100);
        let (items, skills) = Phase4Player::restore(&checkpoint(), &old, Tick(500), 100).unwrap();
        assert_eq!(items.equipment_sources.keys().copied().collect::<Vec<_>>(), vec![100, 102]);
        assert_eq!(
            skills
                .modifier_install_order
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![101, 103]
        );
        assert_eq!(skills.next_install_sequence, 104);
        assert_eq!(items.item_reuse.first().unwrap().until, Tick(507));
        assert!(items.overloaded);
        assert_eq!(items.overload_level, 1);
        assert_eq!(old, timers(100));
        // D71: retained charges and toggles are cleared at re-admission; nothing else changes.
        let mut charged = checkpoint();
        charged.auto_shots = BTreeSet::from([1835]);
        charged.instances.push(ItemInstance {
            creation_origin: None,
            id: super::ItemInstanceId::new(7).unwrap(),
            template_id: 71,
            count: 1,
            location: super::ItemLocation::Paperdoll,
            slot: Some(super::EquipSlot::Rhand),
            enchant: 3,
            augment_id: None,
            charges: 3,
            created_tick: Tick(9),
            revision: 1,
        });
        charged
            .paperdoll
            .insert(super::EquipSlot::Rhand, super::ItemInstanceId::new(7).unwrap());
        let (cleared, _) = Phase4Player::restore(&charged, &old, Tick(500), 100).unwrap();
        assert_eq!(cleared.inventory.len(), 1);
        assert!(cleared.auto_shots.is_empty());
        for item in &charged.instances {
            let restored = cleared.inventory.get(&item.id).unwrap();
            assert_eq!(restored.charges, 0);
            assert_eq!(
                ItemInstance {
                    charges: item.charges,
                    ..restored.clone()
                },
                *item
            );
        }
        assert_eq!(cleared.inventory_revision, charged.inventory_revision);
    }
    #[test]
    fn failed_restore_never_mutates_staged_skills_and_rejects_reordered_phase3() {
        let old = timers(u64::MAX - 2);
        assert!(Phase4Player::restore(&checkpoint(), &old, Tick(0), u64::MAX - 2).is_err());
        assert_eq!(old, timers(u64::MAX - 2));
        let old = timers(100);
        let mut bad = checkpoint();
        bad.equipment_install_order.swap(1, 3);
        assert!(Phase4Player::restore(&bad, &old, Tick(0), 100).is_err());
        assert!(Phase4Player::restore(&checkpoint(), &old, Tick(u64::MAX), 100).is_err());
        assert!(Phase4Player::restore(&checkpoint(), &old, Tick(0), 99).is_err());
        assert_eq!(old, timers(100));
    }
}
