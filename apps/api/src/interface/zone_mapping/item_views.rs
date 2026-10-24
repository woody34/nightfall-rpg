//! Wire projections share the canonical owner/public field boundary with replay.
use super::pb;
use crate::application::AppError;
use crate::domain::zone::item_views::{
    ItemPenaltyView, ItemViewDetail, ItemViewEvent, ItemWeightView, OwnedItem, VisibleEquipment,
};
use crate::domain::zone::phase4::ItemReuse;
use crate::domain::zone::phase4_checkpoint::ItemReuseKey;
use crate::domain::zone::ItemLocation;

pub(super) fn event_to_pb(view: &ItemViewEvent) -> Result<pb::world_event::Event, AppError> {
    view.validate().map_err(anyhow::Error::from)?;
    let entity_id = view.entity.as_uuid().as_bytes().to_vec();
    let incarnation = view.incarnation;
    let tick = view.tick.0;
    Ok(match &view.detail {
        ItemViewDetail::InventorySync {
            baseline_tick,
            inventory,
        } => pb::world_event::Event::InventorySync(pb::InventorySync {
            entity_id,
            incarnation,
            tick,
            inventory_revision: inventory.inventory_revision,
            baseline_tick: baseline_tick.0,
            baseline: true,
            items: inventory.items.iter().map(owned_to_pb).collect(),
            paperdoll: inventory
                .paperdoll
                .iter()
                .map(|(slot, id)| pb::PaperdollEntry {
                    slot: i32::from(*slot as u8).saturating_add(1),
                    instance_id: id.get(),
                })
                .collect(),
            auto_shot_templates: inventory.auto_shots.clone(),
            weight: Some(weight_to_pb(&inventory.weight)),
            penalties: Some(penalty_to_pb(&inventory.penalties)),
            reuse: inventory.reuse.iter().map(reuse_to_pb).collect(),
            rules_hash: inventory.rules_hash.to_vec(),
            ordinary_capacity: inventory.ordinary_capacity,
            quest_capacity: inventory.quest_capacity,
        }),
        ItemViewDetail::InventoryChanged {
            previous_revision,
            inventory_revision,
            added,
            removed,
            updated,
            auto_shots,
            reuse,
        } => pb::world_event::Event::InventoryChanged(pb::InventoryChanged {
            entity_id,
            incarnation,
            tick,
            inventory_revision: *inventory_revision,
            previous_revision: *previous_revision,
            added: added.iter().map(owned_to_pb).collect(),
            removed_ids: removed.iter().map(|id| id.get()).collect(),
            updated: updated.iter().map(owned_to_pb).collect(),
            auto_shot_templates: auto_shots.clone(),
            reuse: reuse.iter().map(reuse_to_pb).collect(),
        }),
        ItemViewDetail::EquipmentChanged { visible } => {
            pb::world_event::Event::EquipmentChanged(pb::EquipmentChanged {
                entity_id,
                incarnation,
                tick,
                visible: visible.iter().map(visible_to_pb).collect(),
            })
        },
        ItemViewDetail::WeightChanged {
            inventory_revision,
            weight,
        } => pb::world_event::Event::WeightChanged(pb::WeightChanged {
            entity_id,
            incarnation,
            tick,
            inventory_revision: *inventory_revision,
            weight: Some(weight_to_pb(weight)),
        }),
        ItemViewDetail::PenaltyChanged {
            inventory_revision,
            penalties,
        } => pb::world_event::Event::PenaltyChanged(pb::PenaltyChanged {
            entity_id,
            incarnation,
            tick,
            inventory_revision: *inventory_revision,
            penalties: Some(penalty_to_pb(penalties)),
        }),
    })
}
pub(super) fn owned_to_pb(item: &OwnedItem) -> pb::OwnedItem {
    pb::OwnedItem {
        instance_id: item.id.get(),
        template_id: item.template_id,
        count: item.count,
        enchant: item.enchant,
        augment_id: item.augment_id,
        location: match item.location {
            ItemLocation::Inventory => pb::ItemLocation::Inventory,
            ItemLocation::Paperdoll => pb::ItemLocation::Paperdoll,
        }
        .into(),
        slot: item
            .slot
            .map_or(0, |slot| i32::from(slot as u8).saturating_add(1)),
        charge_bits: u32::from(item.charges),
        revision: item.revision,
    }
}
pub(super) fn visible_to_pb(item: &VisibleEquipment) -> pb::VisibleEquipment {
    pb::VisibleEquipment {
        slot: i32::from(item.slot as u8).saturating_add(1),
        template_id: item.template_id,
        glow_tier: u32::from(item.glow_tier),
    }
}
pub(super) fn weight_to_pb(weight: &ItemWeightView) -> pb::ItemWeightState {
    pb::ItemWeightState {
        weight: weight.weight,
        max_load: weight.max_load,
        penalty_bonus: weight.penalty_bonus,
        per_mille: weight.per_mille,
    }
}
pub(super) fn penalty_to_pb(penalties: &ItemPenaltyView) -> pb::ItemPenaltyState {
    pb::ItemPenaltyState {
        expertise: u32::from(penalties.expertise),
        weapon_level: u32::from(penalties.weapon_level),
        armor_level: u32::from(penalties.armor_level),
        overload_level: u32::from(penalties.overload_level),
        overloaded: penalties.overloaded,
    }
}
fn reuse_to_pb(reuse: &ItemReuse) -> pb::ItemReuseState {
    let (kind, key) = match reuse.key {
        ItemReuseKey::Template(key) => (pb::ItemReuseKind::Template, key),
        ItemReuseKey::SharedGroup(key) => (pb::ItemReuseKind::SharedGroup, key),
    };
    pb::ItemReuseState {
        kind: kind.into(),
        key,
        until_tick: reuse.until.0,
    }
}
