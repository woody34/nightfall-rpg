//! Final-tick item projections borrow the pre-tick state; no second inventory snapshot.
use super::*;
use crate::domain::zone::item_views::{
    equipment_delta, inventory_delta, visible_equipment, InventoryView, ItemPenaltyView,
    ItemViewDetail, ItemViewEvent, ItemWeightView,
};
use crate::domain::zone::Phase4Snapshot;

impl ZoneState {
    pub(super) fn item_baseline_v6(
        &self,
        tick: Tick,
        id: EntityId,
    ) -> Result<Option<ZoneEvent>, TickError> {
        let Some(items) = &self.phase4 else {
            return Ok(None);
        };
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(TickError::Phase3Invariant)?;
        let inventory = InventoryView::from_player(player, items.rules.hash())
            .map_err(|_| TickError::Phase3Invariant)?;
        Ok(Some(self.item_view_v6(
            tick,
            id,
            ItemViewDetail::InventorySync {
                baseline_tick: tick,
                inventory,
            },
        )?))
    }
    fn item_view_v6(
        &self,
        tick: Tick,
        id: EntityId,
        detail: ItemViewDetail,
    ) -> Result<ZoneEvent, TickError> {
        let view = ItemViewEvent {
            entity: id,
            incarnation: self
                .binding(id)
                .ok_or(TickError::Phase3Invariant)?
                .incarnation,
            tick,
            detail,
        };
        view.validate().map_err(|_| TickError::Phase3Invariant)?;
        Ok(ZoneEvent::ItemView(Box::new(view)))
    }
    pub(super) fn final_items_v6(
        &self,
        tick: Tick,
        before: Option<&Phase4Snapshot>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let (Some(items), Some(before)) = (&self.phase4, before) else {
            return Ok(());
        };
        let admitted: std::collections::BTreeSet<_> = events
            .iter()
            .filter_map(|event| {
                if let ZoneEvent::EntitySpawn { entity, .. } = event {
                    Some(*entity)
                } else {
                    None
                }
            })
            .collect();
        for (id, player) in &items.canonical.players {
            if admitted.contains(id) {
                continue;
            }
            let Some(old) = before.players.get(id) else {
                continue;
            };
            if old == player {
                continue;
            }
            let revision = player.inventory_revision;
            if old.inventory_revision == revision
                && (old.inventory != player.inventory
                    || old.paperdoll != player.paperdoll
                    || old.auto_shots != player.auto_shots
                    || old.item_reuse != player.item_reuse)
            {
                return Err(TickError::Phase3Invariant);
            }
            if old.inventory_revision != revision {
                let previous = InventoryView::from_player(old, before.rules_hash)
                    .map_err(|_| TickError::Phase3Invariant)?;
                let final_view = InventoryView::from_player(player, items.rules.hash())
                    .map_err(|_| TickError::Phase3Invariant)?;
                if let Some(detail) = inventory_delta(&previous, &final_view)
                    .map_err(|_| TickError::Phase3Invariant)?
                {
                    events.push(self.item_view_v6(tick, *id, detail)?);
                }
                let previous =
                    visible_equipment(old, &items.rules).map_err(|_| TickError::Phase3Invariant)?;
                let current = visible_equipment(player, &items.rules)
                    .map_err(|_| TickError::Phase3Invariant)?;
                let visible =
                    equipment_delta(&previous, &current).map_err(|_| TickError::Phase3Invariant)?;
                if !visible.is_empty() {
                    events.push(self.item_view_v6(
                        tick,
                        *id,
                        ItemViewDetail::EquipmentChanged { visible },
                    )?);
                }
                if !events
                    .iter()
                    .any(|e| matches!(e, ZoneEvent::StatsChanged { entity, .. } if entity == id))
                {
                    events.push(self.owner_stats_v5(tick, *id, None)?);
                }
            }
            if (old.weight, old.max_load, old.weight_penalty_bonus)
                != (player.weight, player.max_load, player.weight_penalty_bonus)
            {
                let weight = ItemWeightView::new(
                    player.weight,
                    player.max_load,
                    player.weight_penalty_bonus,
                )
                .map_err(|_| TickError::Phase3Invariant)?;
                events.push(self.item_view_v6(
                    tick,
                    *id,
                    ItemViewDetail::WeightChanged {
                        inventory_revision: revision,
                        weight,
                    },
                )?);
            }
            let penalties = ItemPenaltyView::from(player);
            if penalties != ItemPenaltyView::from(old) {
                events.push(self.item_view_v6(
                    tick,
                    *id,
                    ItemViewDetail::PenaltyChanged {
                        inventory_revision: revision,
                        penalties,
                    },
                )?);
            }
        }
        Ok(())
    }
}
