//! V6 canonical item state. Offline checkpoints pause timers; zone snapshots never do.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::effects::TargetBinding;
use super::item_angles::FacingVector;
use super::phase4_checkpoint::{
    EquipmentSourceKey, ItemReuseKey, SetActivation, StarterKitReceipt,
};
use super::{
    AttackOutcome, CastId, CastInFlight, EntityId, EquipSlot, ItemError, ItemInstance,
    ItemInstanceId, ItemInstanceRange, Phase3Snapshot, Phase4Rules, Tick,
};

/// Source charge kinds. These bits describe the paid charge on the weapon instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotKind {
    /// Physical charge.
    Soul,
    /// Ordinary magical charge.
    Spirit,
    /// Blessed magical charge, preferred without clearing the ordinary bit.
    BlessedSpirit,
}
impl ShotKind {
    /// Canonical bit in `ItemInstance.charges`.
    pub const fn bit(self) -> u8 {
        match self {
            Self::Soul => 1,
            Self::Spirit => 2,
            Self::BlessedSpirit => 4,
        }
    }
}

/// An absolute reuse deadline, in key order. Lists preserve duplicate detection in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemReuse {
    pub key: ItemReuseKey,
    pub until: Tick,
}

/// Item ownership and history. Stat calculators and lookup indexes are rebuilt caches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase4Player {
    pub inventory: BTreeMap<ItemInstanceId, ItemInstance>,
    pub paperdoll: BTreeMap<EquipSlot, ItemInstanceId>,
    pub auto_shots: BTreeSet<u32>,
    pub item_reuse: Vec<ItemReuse>,
    pub equipment_sources: BTreeMap<u64, EquipmentSourceKey>,
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
    pub starter_kit_receipt: StarterKitReceipt,
    pub inventory_revision: u64,
}

/// Frozen ownership of a paid charge, independent of the actor's later selection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum ItemActionBinding {
    Cast { id: CastId },
    Attack { attacker: EntityId, start: Tick },
}

/// Canonical ordered binding list; enum/tuple keys cannot be JSON object keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ChargeBinding {
    pub weapon: ItemInstanceId,
    pub shot: ShotKind,
    pub action: ItemActionBinding,
}

/// Consumed item identity is evidence even when its last unit has disappeared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemPayment {
    pub instance: ItemInstanceId,
    pub template: u32,
    pub count: u64,
}

/// Item-granted cast authority and its accepted operation identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemCastOrigin {
    pub instance: ItemInstanceId,
    pub template: u32,
    /// Actual clicked-stack debit; zero requires the source to remain owned.
    pub count: u64,
    pub skill_ordinal: u32,
    pub receipt: super::phase4_projection::ItemReceiptRef,
}

/// A simultaneous item-origin lane owns its complete cast; a main cast's shot context
/// refers to the identical canonical Phase3 cast. IDs use one shared allocator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemCastState {
    pub caster: TargetBinding,
    pub cast: CastInFlight,
    pub origin: Option<ItemCastOrigin>,
    pub simultaneous: bool,
    pub weapon: Option<ItemInstanceId>,
    pub shots: u8,
}

/// Shield outcome is independent of critical and caller-side truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShieldOutcome {
    /// No eligible block or failed block roll.
    None,
    /// Ordinary defence addition.
    Normal,
    /// Formula early return, before later caller transformations.
    Perfect,
}

/// A dual subhit's result can be prepared before its deadline (I3). Recipient hooks
/// occur only at the deadline and do not rerun the saved formula draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PreparedItemHit {
    pub outcome: AttackOutcome,
    pub shield: ShieldOutcome,
    pub damage: u32,
}

/// One ordered target/impact in the canonical weapon action bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PlannedItemHit {
    pub target: TargetBinding,
    pub at: Tick,
    pub prepared: Option<PreparedItemHit>,
}

/// Projectile payment and weapon identity survive save/restore without consulting gear
/// equipped after the action started. Cancellation does not refund accepted costs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemAttackPlan {
    pub attacker: TargetBinding,
    pub start: Tick,
    pub ready: Tick,
    pub weapon: Option<ItemInstanceId>,
    pub shots: u8,
    pub arrow: Option<ItemPayment>,
    pub hits: Vec<PlannedItemHit>,
    pub next_hit: u32,
}

/// Snapshot10 sidecar. Never synthesized when reading a legacy epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase4Snapshot {
    pub formula_revision: u32,
    pub timer_policy_version: u32,
    pub rules_hash: [u8; 32],
    pub allocator: ItemInstanceRange,
    pub players: BTreeMap<EntityId, Phase4Player>,
    pub facing: BTreeMap<EntityId, FacingVector>,
    pub item_casts: BTreeMap<CastId, ItemCastState>,
    pub attack_plans: BTreeMap<EntityId, ItemAttackPlan>,
    pub charge_bindings: Vec<ChargeBinding>,
}

impl Phase4Snapshot {
    /// Empty epoch with an explicitly reserved allocator range.
    pub fn new(rules: &Phase4Rules, allocator: ItemInstanceRange) -> Self {
        Self {
            rules_hash: rules.hash(),
            formula_revision: rules.parts().formula_revision,
            timer_policy_version: 1,
            allocator,
            players: BTreeMap::new(),
            facing: BTreeMap::new(),
            item_casts: BTreeMap::new(),
            attack_plans: BTreeMap::new(),
            charge_bindings: Vec::new(),
        }
    }

    /// Cross-sidecar allocation checks. Inventory/fold and combat-life checks are also
    /// required before restoration; this method alone cannot authorize gameplay.
    pub fn validate_allocations(
        &self,
        rules: &Phase4Rules,
        phase3: &Phase3Snapshot,
    ) -> Result<(), ItemError> {
        if self.rules_hash != rules.hash()
            || rules.parts().phase3_hash != phase3.rules_hash
            || self.formula_revision != rules.parts().formula_revision
            || self.timer_policy_version != 1
        {
            return Err(ItemError("Phase4 rules hash mismatch"));
        }
        let mut instances = BTreeSet::new();
        let mut sequences: BTreeSet<_> = phase3
            .entities
            .values()
            .flat_map(|entity| entity.modifier_install_order.keys().copied())
            .collect();
        let (start, next, end) = self.allocator.bounds();
        for (id, player) in &self.players {
            if phase3
                .entities
                .get(id)
                .is_none_or(|entity| entity.player.is_none())
            {
                return Err(ItemError("Phase4 player without Phase3 player"));
            }
            if player.auto_shots.iter().any(|template| {
                rules
                    .parts()
                    .capabilities
                    .get(template)
                    .is_none_or(|capability| {
                        !capability.admitted || !capability.use_blockers.is_empty()
                    })
                    || rules.parts().items.get(template).is_none_or(|item| {
                        item.etcitem_type != "SHOT"
                            || !matches!(
                                item.handler.as_str(),
                                "SoulShots" | "SpiritShot" | "BlessedSpiritShot"
                            )
                    })
            }) {
                return Err(ItemError("unsupported automatic shot template"));
            }
            for (key, item) in &player.inventory {
                item.validate()?;
                if *key != item.id
                    || !instances.insert(*key)
                    || (key.get() >= start && key.get() < end && key.get() >= next)
                {
                    return Err(ItemError("duplicate/unallocated inventory identity"));
                }
            }
            for sequence in player.equipment_sources.keys() {
                if *sequence == 0
                    || *sequence >= phase3.next_install_sequence
                    || !sequences.insert(*sequence)
                {
                    return Err(ItemError("duplicate/unallocated equipment sequence"));
                }
            }
        }
        if phase3
            .entities
            .values()
            .filter(|entity| entity.player.is_some())
            .count()
            != self.players.len()
            || !phase3.entities.keys().eq(self.facing.keys())
        {
            return Err(ItemError("Phase4 combatant sidecar mismatch"));
        }
        let mut casts: BTreeSet<_> = phase3
            .entities
            .values()
            .filter_map(|entity| match &entity.cast {
                super::CastState::Idle => None,
                super::CastState::Casting(cast) => Some(cast.id),
            })
            .collect();
        for (id, context) in &self.item_casts {
            if *id != context.cast.id || id.0 == 0 || id.0 >= phase3.next_cast_id {
                return Err(ItemError("invalid item cast allocation"));
            }
            if context.simultaneous {
                if context.origin.is_none() || !casts.insert(*id) { return Err(ItemError("duplicate item cast")); }
            } else if !phase3.entities.get(&context.caster.entity).is_some_and(|entity| {
                matches!(&entity.cast, super::CastState::Casting(cast) if cast == &context.cast)
            }) { return Err(ItemError("item context without matching main cast")); }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};

    #[test]
    fn snapshot_binds_rules_and_requires_every_combatants_facing_and_inventory() {
        let mut phase3 =
            Phase3Snapshot::new(load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap());
        let mut parts = crate::infrastructure::item_data::generated_test_rules()
            .parts()
            .clone();
        parts.phase3_hash = phase3.rules_hash;
        let rules = Phase4Rules::try_new(parts).unwrap();
        let mut snapshot = Phase4Snapshot::new(&rules, ItemInstanceRange::new(1, 1, 100).unwrap());
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert!(encoded.get("rules").is_none());
        assert_eq!(serde_json::from_value::<Phase4Snapshot>(encoded.clone()).unwrap(), snapshot);
        let mut smuggled = encoded;
        smuggled
            .as_object_mut()
            .unwrap()
            .insert("rules".into(), serde_json::Value::Null);
        assert!(serde_json::from_value::<Phase4Snapshot>(smuggled).is_err());
        snapshot.validate_allocations(&rules, &phase3).unwrap();
        snapshot.rules_hash[0] ^= 1;
        assert!(snapshot.validate_allocations(&rules, &phase3).is_err());
        snapshot.rules_hash = rules.hash();
        let id = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let mut npc = super::super::phase3_entity::tests::blank();
        npc.player = None;
        phase3.entities.insert(id, npc);
        assert!(snapshot.validate_allocations(&rules, &phase3).is_err());
        snapshot.facing.insert(id, FacingVector::EAST);
        snapshot.validate_allocations(&rules, &phase3).unwrap();
        phase3
            .entities
            .insert(id, super::super::phase3_entity::tests::blank());
        assert!(snapshot.validate_allocations(&rules, &phase3).is_err());
        assert_eq!(
            ShotKind::Soul.bit() | ShotKind::Spirit.bit() | ShotKind::BlessedSpirit.bit(),
            7
        );
    }
}

/// Runtime-only resolved rules are never serialized with canonical state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Phase4Zone {
    pub canonical: Phase4Snapshot,
    pub rules: std::sync::Arc<Phase4Rules>,
}
