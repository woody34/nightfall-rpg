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
        if admitted_in(&snapshot, character).is_none() {
            return Ok(None);
        }
        let merchant = EntityId::from_uuid(merchant);
        let is_npc = snapshot
            .entities
            .iter()
            .any(|e| e.id == merchant && e.kind == EntityKind::Npc);
        let Some(npc_id) = snapshot.pinned_npc_id(merchant).filter(|_| is_npc) else {
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
}
