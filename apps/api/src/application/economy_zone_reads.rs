//! The zone runtime **read** port of the Phase 5 read RPCs (story 4.6, design §13.4) over the
//! running zone actor: admitted state, peace square and vendor state at the zone's current tick.
//! It reads an actor snapshot only; nothing is mutated or enqueued.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use super::use_cases::economy_queries::{AdmittedView, EconomyZoneReads, VendorState};
use super::zone_actor::ZoneHandle;
use super::AppError;
use crate::domain::zone::{EntityId, EntityKind, Phase5Rules, ZoneSnapshot};

/// The zone read port over one zone handle (Phase 5 runs a single test zone).
pub struct ZoneEconomyReads {
    zone: ZoneHandle,
    rules: Arc<Phase5Rules>,
}

impl ZoneEconomyReads {
    /// A read port over `zone` and the zone's V7 `rules`. Merchant entities resolve through the
    /// merchant fixtures (`Phase5Rules::merchant_fixture`, ruling P5-D29), as the zone's own
    /// vendor intents do. Monsters keep their spawn-slot `pinned_npc_id`; a monster is never a
    /// merchant.
    #[must_use]
    pub const fn new(zone: ZoneHandle, rules: Arc<Phase5Rules>) -> Self {
        Self { zone, rules }
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
        vendor_in(&snapshot, character, merchant, |entity| {
            merchant_fixture_in(&snapshot, &self.rules, entity)
        })
    }
}

/// P5-D29: the pinned NPC of a merchant fixture entity: a live non-combat, non-attackable NPC
/// whose name and exact position are a `merchant_fixtures.toml` row (the zone's
/// `merchant_npc_v7` check). Any other entity is not a merchant.
#[must_use]
pub fn merchant_fixture_in(
    snapshot: &ZoneSnapshot,
    rules: &Phase5Rules,
    entity: EntityId,
) -> Option<u32> {
    let npc = snapshot.entities.iter().find(|e| e.id == entity)?;
    let fixture = npc.kind == EntityKind::Npc
        && npc.combat.is_none()
        && !npc.targeting.attackable
        && !npc.targeting.dead;
    fixture.then(|| rules.merchant_fixture(&npc.name, npc.pos))?
}

/// `ListBuylist`'s merchant check over one snapshot: an admitted caller, an NPC entity, and its
/// pinned NPC id from `resolve` (the merchant fixtures in production, P5-D29). Anything else is
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
        // The production resolver: an NPC that is not a merchant fixture is not a merchant.
        let rules = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        assert!(vendor_in(&snap, caller, Uuid::from_u128(2), |e| {
            merchant_fixture_in(&snap, &rules, e)
        })
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

    /// The embedded rules with the fixture row `(35363, "Alfred", 12, 10)` bound.
    fn rules_with_alfred() -> Phase5Rules {
        let base = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        let mut parts = base.parts().clone();
        // Replace any bound row of the npc (the embedded table has 35363 as restock_merchant).
        let rows = &mut parts.merchant_fixtures.merchants;
        rows.retain(|row| row.npc != 35_363);
        rows.push(crate::domain::zone::phase5_rules::MerchantFixture {
            npc: 35_363,
            name: "Alfred".into(),
            x: 12,
            y: 10,
        });
        rows.sort_by_key(|row| row.npc);
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let mut rules = Phase5Rules::try_new(parts).unwrap();
        rules.bind_items(&bundle.items).unwrap();
        rules.bind_skills(&bundle.skills).unwrap();
        rules
    }

    #[test]
    fn a_merchant_is_a_fixture_npc_by_name_and_exact_position() {
        let rules = rules_with_alfred();
        let mut snap = snapshot();
        let merchant = EntityId::from_uuid(Uuid::from_u128(2));
        let at = |snap: &mut ZoneSnapshot, name: &str, x: i32| {
            let npc = snap.entities.iter_mut().find(|e| e.id == merchant).unwrap();
            npc.name = name.into();
            npc.pos = Vec2Fixed::from_tiles(x, 10);
        };
        at(&mut snap, "Alfred", 12);
        assert_eq!(merchant_fixture_in(&snap, &rules, merchant), Some(35_363));
        let caller = Uuid::from_u128(1);
        let state =
            vendor_in(&snap, caller, merchant.as_uuid(), |e| merchant_fixture_in(&snap, &rules, e))
                .unwrap()
                .unwrap();
        assert_eq!(state.merchant_npc_id, 35_363);
        // Another tile or another name is not the fixture.
        at(&mut snap, "Alfred", 11);
        assert_eq!(merchant_fixture_in(&snap, &rules, merchant), None);
        at(&mut snap, "Antonio", 12);
        assert_eq!(merchant_fixture_in(&snap, &rules, merchant), None);
        // An attackable or dead NPC, or a player, at the fixture tile is not a merchant.
        at(&mut snap, "Alfred", 12);
        for change in [0, 1, 2] {
            let mut probe = snap.clone();
            let npc = probe
                .entities
                .iter_mut()
                .find(|e| e.id == merchant)
                .unwrap();
            match change {
                0 => npc.targeting.attackable = true,
                1 => npc.targeting.dead = true,
                _ => npc.kind = EntityKind::Player,
            }
            assert_eq!(merchant_fixture_in(&probe, &rules, merchant), None, "{change}");
        }
    }
}
