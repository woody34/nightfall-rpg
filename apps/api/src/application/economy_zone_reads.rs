//! The zone runtime **read** port of the Phase 5 read RPCs (story 4.6, design §13.4) over the
//! running zone actor: admitted state, peace square and vendor state at the zone's current tick.
//! It reads an actor snapshot only; nothing is mutated or enqueued.

use async_trait::async_trait;
use uuid::Uuid;

use super::use_cases::economy_queries::{AdmittedView, EconomyZoneReads, VendorState};
use super::zone_actor::ZoneHandle;
use super::AppError;
use crate::domain::zone::{EntityId, EntityKind, ZoneSnapshot};

/// The zone read port over one zone handle (Phase 5 runs a single test zone).
pub struct ZoneEconomyReads {
    zone: ZoneHandle,
}

impl ZoneEconomyReads {
    /// A read port over `zone`. Merchant entities resolve to their pinned NPC id through the
    /// spawn-slot binding (`ZoneSnapshot::pinned_npc_id`, P5-Q14), as the zone's own vendor
    /// intents do.
    #[must_use]
    pub const fn new(zone: ZoneHandle) -> Self {
        Self { zone }
    }

    async fn snapshot(&self) -> Result<ZoneSnapshot, AppError> {
        self.zone
            .snapshot()
            .await
            .map_err(|_| AppError::Unavailable("zone runtime is unavailable".into()))
    }
}

/// Whether the character is admitted in the snapshot, and inside its peace square.
#[must_use]
pub fn admitted_in(snapshot: &ZoneSnapshot, character: Uuid) -> Option<AdmittedView> {
    let id = EntityId::from_uuid(character);
    let entity = snapshot
        .entities
        .iter()
        .find(|e| e.id == id && e.kind == EntityKind::Player)?;
    let in_peace_square = snapshot.phase3.as_ref().is_some_and(|phase| {
        phase
            .rules
            .geometry()
            .peace_square(snapshot.bounds)
            .is_ok_and(|square| square.contains(entity.pos))
    });
    Some(AdmittedView { in_peace_square })
}

#[async_trait]
impl EconomyZoneReads for ZoneEconomyReads {
    async fn admitted(&self, character: Uuid) -> Result<Option<AdmittedView>, AppError> {
        Ok(admitted_in(&self.snapshot().await?, character))
    }

    async fn vendor(
        &self,
        character: Uuid,
        merchant: Uuid,
    ) -> Result<Option<VendorState>, AppError> {
        let snapshot = self.snapshot().await?;
        vendor_in(&snapshot, character, merchant, |entity| snapshot.pinned_npc_id(entity))
    }
}

/// `ListBuylist`'s merchant check over one snapshot (P5-Q14): an admitted caller, an NPC entity,
/// and its pinned NPC id from `resolve` (the spawn-slot binding in production). Anything else is
/// `None`, which the use case answers `NOT_FOUND`.
pub fn vendor_in(
    snapshot: &ZoneSnapshot,
    character: Uuid,
    merchant: Uuid,
    resolve: impl Fn(EntityId) -> Option<u32>,
) -> Result<Option<VendorState>, AppError> {
    if admitted_in(snapshot, character).is_none() {
        return Ok(None);
    }
    let merchant = EntityId::from_uuid(merchant);
    let is_npc = snapshot
        .entities
        .iter()
        .any(|e| e.id == merchant && e.kind == EntityKind::Npc);
    let Some(npc_id) = resolve(merchant).filter(|_| is_npc) else {
        return Ok(None);
    };
    let Some(block) = snapshot.phase5.as_ref() else {
        return Err(AppError::Unavailable("vendor state needs a V7 zone".into()));
    };
    let stock = block
        .stock
        .iter()
        .map(|(key, stock)| (*key, (u64::try_from(stock.count).unwrap_or(0), stock.max)))
        .collect();
    let refund_tab = block
        .players
        .get(&EntityId::from_uuid(character))
        .map(|player| player.refund.clone())
        .unwrap_or_default();
    Ok(Some(VendorState {
        merchant_npc_id: npc_id,
        state_tick: snapshot.tick.0,
        stock,
        refund_tab,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::zone::*;

    fn snapshot() -> ZoneSnapshot {
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let mut snapshot = ZoneState::new_phase5(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            crate::infrastructure::rules_data::load_rules(
                &crate::infrastructure::rules_data::RulesSource::embedded(),
            )
            .unwrap()
            .rules,
            crate::infrastructure::class_data::load_classes(
                &crate::infrastructure::class_data::ClassSource::embedded(),
            )
            .unwrap()
            .registry,
            (*bundle.skills).clone(),
            bundle.items.clone(),
            ItemInstanceRange::new(1, 1, 100).unwrap(),
            crate::infrastructure::economy_data::embedded_phase5_rules().unwrap(),
        )
        .unwrap()
        .snapshot();
        for (n, kind) in [
            (1_u128, EntityKind::Player),
            (2, EntityKind::Npc),
            (3, EntityKind::Player),
        ] {
            snapshot.entities.push(Entity {
                id: EntityId::from_uuid(Uuid::from_u128(n)),
                kind,
                name: String::new(),
                pos: Vec2Fixed::from_tiles(10, 10),
                dest: None,
                speed: Speed::from_milli_tiles_per_tick(0),
                generation: SessionGeneration::default(),
                targeting: TargetingState::default(),
                combat: None,
                ai: None,
            });
        }
        snapshot
    }

    /// The binding of the test: entities 2 (an NPC) and 3 (a player) both resolve to Alfred.
    fn resolve(entity: EntityId) -> Option<u32> {
        [2_u128, 3]
            .contains(&entity.as_uuid().as_u128())
            .then_some(35_363)
    }

    #[test]
    fn the_merchant_check_needs_an_admitted_caller_an_npc_and_a_pinned_id() {
        let snap = snapshot();
        let caller = Uuid::from_u128(1);
        let state = vendor_in(&snap, caller, Uuid::from_u128(2), resolve)
            .unwrap()
            .unwrap();
        assert_eq!(state.merchant_npc_id, 35_363);
        assert_eq!(state.state_tick, snap.tick.0);
        // A player entity with a binding is not a merchant.
        assert!(vendor_in(&snap, caller, Uuid::from_u128(3), resolve)
            .unwrap()
            .is_none());
        // An NPC without a binding, and an unknown entity, are not merchants.
        assert!(vendor_in(&snap, caller, Uuid::from_u128(2), |_| None)
            .unwrap()
            .is_none());
        assert!(vendor_in(&snap, caller, Uuid::from_u128(9), resolve)
            .unwrap()
            .is_none());
        // A caller who is not admitted reads nothing.
        assert!(vendor_in(&snap, Uuid::from_u128(4), Uuid::from_u128(2), resolve)
            .unwrap()
            .is_none());
        // The production resolver: an NPC entity without a spawn slot has no pinned id.
        assert!(vendor_in(&snap, caller, Uuid::from_u128(2), |e| snap.pinned_npc_id(e))
            .unwrap()
            .is_none());
        // Before V7 there is no vendor state.
        let mut v6 = snap.clone();
        v6.phase5 = None;
        assert!(matches!(
            vendor_in(&v6, caller, Uuid::from_u128(2), resolve),
            Err(AppError::Unavailable(_))
        ));
    }
}
