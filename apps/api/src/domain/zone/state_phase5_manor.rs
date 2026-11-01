//! V7 manor seeding and harvesting (story 3.6, design §10.6; B1, E13): the `Seed` and
//! `Harvester` item handlers (pinned `Seed.java:42-90`, `Harvester.java:45-70`) and the
//! `Sow` / `Harvesting` instant effects of skills 2097 / 2098 (`Sow.java:51-125`,
//! `Harvesting.java:48-119`, `L2Attackable.java:1257-1294`), admitted by the P5-D19 V7 skill
//! view. Draws #12 and #13 are `chance_bound(basic, 99)` (pinned `Rnd.nextInt(99) < basic`).

use crate::domain::zone::effects::TargetBinding;
use crate::domain::zone::item_inventory::{inventory_totals, InventoryBoundary, InventoryEdit};
use crate::domain::zone::phase4_projection::{ItemAuditEvidence, ItemEvidenceValue, ItemOperation};
use crate::domain::zone::phase5::SeedBinding;
use crate::domain::zone::phase5_rules::ManorCrop;
use crate::domain::zone::{
    CastId, EntityId, EntityKind, ItemCreationOrigin, ItemInstance, ItemLocation, RejectReason,
    SkillOutcome, Tick, ZoneEvent, ZoneState,
};

use super::phase5_pickup::CharacterRow;

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

#[allow(clippy::needless_pass_by_value)] // map_err adapter.
fn fault<E>(_: E) -> super::TickError {
    super::TickError::Phase5Invariant
}

/// `Sow.calcSuccess` (`:89-123`): 90 (alternative 20), −5 per level outside
/// `[seed − 5, seed + 5]`, −5 per player/target level beyond 5; the pinned `Math.max(.., 1)`
/// result is discarded, so there is no floor.
#[must_use]
pub(super) fn sow_basic(seed_level: i64, alternative: bool, player: i64, target: i64) -> i64 {
    let mut basic: i64 = if alternative { 20 } else { 90 };
    let (low, high) = (seed_level.saturating_sub(5), seed_level.saturating_add(5));
    if target < low {
        basic = basic.saturating_sub(low.saturating_sub(target).saturating_mul(5));
    }
    if target > high {
        basic = basic.saturating_sub(target.saturating_sub(high).saturating_mul(5));
    }
    let diff = player.saturating_sub(target).saturating_abs();
    if diff > 5 {
        basic = basic.saturating_sub(diff.saturating_sub(5).saturating_mul(5));
    }
    basic
}

/// `Harvesting.calcSuccess` (`:98-119`): 100, −5 per level difference beyond 5, floor 1.
#[must_use]
pub(super) fn harvest_basic(player: i64, target: i64) -> i64 {
    let diff = player.saturating_sub(target).saturating_abs();
    let basic = if diff > 5 {
        100_i64.saturating_sub(diff.saturating_sub(5).saturating_mul(5))
    } else {
        100
    };
    basic.max(1)
}

/// `L2Attackable.setSeeded` (`:1257-1283`): the product of the NPC's strong-type
/// multipliers (skills 4303..4310), plus `max(0, npc − seed − 5)`, times `RateDropManor`.
#[must_use]
pub(super) fn harvest_count(
    strong_skills: &[u32],
    multipliers: &[crate::domain::zone::phase5_rules::StrongTypeMultiplier],
    npc_level: i64,
    seed_level: i64,
    rate_drop_manor: u64,
) -> u64 {
    let mut count: u64 = 1;
    for skill in strong_skills {
        if let Some(m) = multipliers.iter().find(|m| m.skill == *skill) {
            count = count.saturating_mul(u64::from(m.multiplier));
        }
    }
    let bonus = npc_level
        .saturating_sub(seed_level)
        .saturating_sub(5)
        .max(0);
    count
        .saturating_add(u64::try_from(bonus).unwrap_or(0))
        .saturating_mul(rate_drop_manor)
}

impl ZoneState {
    fn manor_crop_of_seed(&self, seed: u32) -> Option<ManorCrop> {
        self.phase5.as_ref().and_then(|p| {
            p.rules
                .parts()
                .manor
                .crops
                .iter()
                .find(|c| c.seed == seed)
                .cloned()
        })
    }

    fn entity_level(&self, id: EntityId) -> Option<i64> {
        self.entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .map(|c| i64::from(c.sheet.level()))
    }

    /// The `Seed` handler checks (§10.6, `Seed.java:42-90`): a living, non-raid monster not
    /// yet *seeded* (pinned `isSeeded()`, the success flag: an unsown binding is replaced);
    /// a seed of the zone's castle area (the hashed fixture `castle_area`, E28). Every
    /// failure refuses `NOT_SEEDABLE`. Nothing changes here: [`Self::seed_bind_v7`] binds
    /// once the Sowing cast has started, so a refused use changes nothing (I1).
    pub(super) fn seed_use_v7(
        &self,
        caster: EntityId,
        seed: u32,
        target: Option<EntityId>,
    ) -> Result<(), RejectReason> {
        let target = target.ok_or(RejectReason::NotSeedable)?;
        let crop = self
            .manor_crop_of_seed(seed)
            .ok_or(RejectReason::NotSeedable)?;
        let phase5 = self
            .phase5
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let area = phase5.rules.parts().manor.fixture_gludio.castle_area;
        let raid = self
            .pinned_npc(target)
            .and_then(|pinned| phase5.rules.reward_npc(pinned))
            .is_some_and(|row| row.is_raid);
        let living_monster = self.entities.get(&target).is_some_and(|e| {
            e.kind == EntityKind::Npc
                && e.ai.is_some()
                && e.targeting.attackable
                && !e.targeting.dead
        });
        let sidecar = phase5.canonical.npcs.get(&target);
        if !living_monster || raid || sidecar.is_none_or(|n| n.seeded) || crop.castle != area {
            return Err(RejectReason::NotSeedable);
        }
        let _ = caster;
        Ok(())
    }

    /// `target.setSeeded(seed, activeChar)` (`L2Attackable.java:1289-1294`): while not seeded,
    /// the binding is (re)set to this seed and seeder.
    pub(super) fn seed_bind_v7(&mut self, caster: EntityId, seed: u32, target: EntityId) {
        if let Some(npc) = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.npcs.get_mut(&target))
        {
            if !npc.seeded {
                npc.seed = Some(SeedBinding {
                    seed,
                    seeder: caster,
                });
            }
        }
    }

    /// The `Harvester` handler: a dead monster target (`Harvester.java:56-62`).
    pub(super) fn harvester_use_v7(&self, target: Option<EntityId>) -> Result<(), RejectReason> {
        let dead_monster = target
            .and_then(|id| self.entities.get(&id))
            .is_some_and(|e| e.kind == EntityKind::Npc && e.ai.is_some() && e.targeting.dead);
        if dead_monster {
            Ok(())
        } else {
            Err(RejectReason::HarvestNotAllowed)
        }
    }

    /// `Sow` (§10.6): guard (alive, `canBeSown`, not seeded, the caster is the seeder), the
    /// seed consumed (1) before draw #12, then on success `seeded` and the harvest
    /// `(crop, Π strong-type multipliers + max(0, npc − seed − 5)) × RateDropManor`.
    pub(super) fn sow_v7(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        cast_id: CastId,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillOutcome, super::TickError> {
        let Some(phase5) = self.phase5.as_ref() else {
            return Ok(SkillOutcome::Miss);
        };
        let rules = std::sync::Arc::clone(&phase5.rules);
        let Some(sidecar) = phase5.canonical.npcs.get(&target.entity).cloned() else {
            return Ok(SkillOutcome::Miss);
        };
        let pinned = self.pinned_npc(target.entity);
        let can_be_sown = pinned
            .and_then(|id| rules.drop_npc(id))
            .is_some_and(|row| row.can_be_sown);
        let alive = self
            .entities
            .get(&target.entity)
            .is_some_and(|e| e.kind == EntityKind::Npc && !e.targeting.dead);
        let Some(binding) = sidecar.seed else {
            return Ok(SkillOutcome::Miss);
        };
        if !alive || !can_be_sown || sidecar.seeded || binding.seeder != caster.entity {
            return Ok(SkillOutcome::Miss);
        }
        let Some(crop) = self.manor_crop_of_seed(binding.seed) else {
            return Ok(SkillOutcome::Miss);
        };
        if !self
            .consume_one_v7(
                tick,
                caster.entity,
                binding.seed,
                &format!("sow:{}", cast_id.0),
                events,
            )
            .map_err(fault)?
        {
            return Ok(SkillOutcome::Miss);
        }
        // z-review-6 P3-1: pinned `target.getAI().setIntention(AI_INTENTION_IDLE)` runs at the
        // end of every Sow that consumed its seed, sown or not.
        self.set_intention(tick, target.entity, crate::domain::zone::ai::Intention::Idle, events);
        let player = self.entity_level(caster.entity).unwrap_or(1);
        let npc = self.entity_level(target.entity).unwrap_or(1);
        let basic = sow_basic(i64::from(crop.level), crop.alternative, player, npc);
        // Draw #12 is taken whatever `basic` is (P3); a basic ≤ 0 never succeeds.
        let success = self
            .chance_bound(u64::try_from(basic.max(0)).unwrap_or(0), 99)
            .map_err(fault)?;
        if !success {
            return Ok(SkillOutcome::Resist);
        }
        let manor = &rules.parts().manor.skills;
        let strong = pinned
            .and_then(|id| rules.drop_npc(id))
            .map(|row| row.harvest_skills.clone())
            .unwrap_or_default();
        let count = harvest_count(
            &strong,
            &manor.strong_type_multipliers,
            npc,
            i64::from(crop.level),
            manor.rate_drop_manor,
        );
        if let Some(npc) = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.npcs.get_mut(&target.entity))
        {
            npc.seeded = true;
            npc.harvest = Some((crop.crop, count));
        }
        Ok(SkillOutcome::Hit)
    }

    /// `Harvesting` (§10.6): only the seeder, a seeded corpse, draw #13; success takes the
    /// harvest as a fresh crop grant (`harvest` row, cause `harvest:<cast_id>`); a failure
    /// keeps it for a retry.
    pub(super) fn harvesting_v7(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        cast_id: CastId,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillOutcome, super::TickError> {
        let Some(sidecar) = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.npcs.get(&target.entity))
            .cloned()
        else {
            return Ok(SkillOutcome::Miss);
        };
        let dead = self
            .entities
            .get(&target.entity)
            .is_some_and(|e| e.kind == EntityKind::Npc && e.targeting.dead);
        if !dead || sidecar.seed.is_none_or(|b| b.seeder != caster.entity) || !sidecar.seeded {
            return Ok(SkillOutcome::Miss);
        }
        let player = self.entity_level(caster.entity).unwrap_or(1);
        let npc = self.entity_level(target.entity).unwrap_or(1);
        let basic = harvest_basic(player, npc);
        if !self
            .chance_bound(u64::try_from(basic).unwrap_or(1), 99)
            .map_err(fault)?
        {
            return Ok(SkillOutcome::Resist);
        }
        let Some((crop, count)) = sidecar.harvest else {
            return Ok(SkillOutcome::Miss);
        };
        if let Some(npc) = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.npcs.get_mut(&target.entity))
        {
            npc.harvest = None;
        }
        self.grant_fresh_v7(
            tick,
            caster.entity,
            crop,
            count,
            &format!("harvest:{}", cast_id.0),
            events,
        )
        .map_err(fault)?;
        Ok(SkillOutcome::Hit)
    }

    /// Consume one `template` from `owner` (`destroyItemByItemId("Consume", .., 1)`): a V6
    /// Consume aggregate in the owner's projection with an automatic `cause`. `false` when the
    /// owner holds none (pinned returns without the roll).
    fn consume_one_v7(
        &mut self,
        tick: Tick,
        owner: EntityId,
        template: u32,
        cause: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<bool, RejectReason> {
        let changed = events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == owner));
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        if !player.inventory.values().any(|i| i.template_id == template) {
            return Ok(false);
        }
        let revision = if changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let mutation = crate::domain::zone::item_inventory::InventoryMutation::prepare(
            &player.inventory,
            items.canonical.allocator,
            &[InventoryEdit::TakeTemplate {
                template,
                count: 1,
                preferred: None,
            }],
            &items.rules,
            crate::domain::zone::item_inventory::InventoryLimits::from_player(player),
            InventoryBoundary {
                tick,
                revision,
                origin: ItemCreationOrigin {
                    zone: self.seed.zone,
                    epoch: self.seed.epoch,
                },
            },
        )
        .map_err(invalid)?;
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get_mut(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        player.inventory = mutation.inventory;
        player.weight = mutation.weight;
        player.inventory_revision = revision;
        self.rebuild_v5(owner).map_err(invalid)?;
        let index = self.economy_projection(tick, owner, events)?;
        let Some(ZoneEvent::ItemCheckpoint(projection)) = events.get_mut(index) else {
            return Err(RejectReason::InvalidLoad);
        };
        for change in mutation.changes {
            let count_delta = i64::try_from(change.before.as_ref().map_or(0, |i| i.count))
                .map_err(invalid)?
                .saturating_sub(
                    i64::try_from(change.after.as_ref().map_or(0, |i| i.count)).map_err(invalid)?,
                )
                .saturating_neg();
            super::item_commands::append_item_consumption(
                projection,
                change,
                count_delta,
                // P5-D26: one `economy` contribution `{count_delta: Signed, cause}`; an
                // automatic debit (Sow) has no receipt.
                ItemAuditEvidence {
                    family: "economy".into(),
                    fields: std::collections::BTreeMap::from([
                        ("count_delta".to_owned(), ItemEvidenceValue::Signed(count_delta)),
                        ("cause".to_owned(), ItemEvidenceValue::Text(cause.to_owned())),
                    ]),
                },
            )?;
        }
        Ok(true)
    }

    /// A fresh grant (or a merge into the stack) of `count` × `template` to `owner`, with its
    /// `harvest` row (cause) in the owner's projection; non-stackables are `count` instances.
    #[allow(clippy::too_many_arguments)] // The grant's explicit inputs.
    fn grant_fresh_v7(
        &mut self,
        tick: Tick,
        owner: EntityId,
        template: u32,
        count: u64,
        cause: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let changed = events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == owner));
        let origin = ItemCreationOrigin {
            zone: self.seed.zone,
            epoch: self.seed.epoch,
        };
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let rules = std::sync::Arc::clone(&items.rules);
        let stackable = rules
            .parts()
            .items
            .get(&template)
            .ok_or(RejectReason::InvalidLoad)?
            .stackable;
        let player = items
            .canonical
            .players
            .get(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        let revision = if changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let existing = stackable
            .then(|| {
                player.inventory.values().find(|i| {
                    i.template_id == template
                        && i.enchant == 0
                        && i.augment_id.is_none()
                        && i.charges == 0
                })
            })
            .flatten()
            .cloned();
        let mut rows = Vec::new();
        if let Some(before) = existing {
            let mut after = before.clone();
            after.count = after.count.saturating_add(count);
            after.revision = revision;
            rows.push((Some(before), after, i64::try_from(count).map_err(invalid)?));
        } else {
            for _ in 0..(if stackable { 1 } else { count }) {
                let id = items.canonical.allocator.allocate().map_err(invalid)?;
                let after = ItemInstance {
                    creation_origin: Some(origin),
                    id,
                    template_id: template,
                    count: if stackable { count } else { 1 },
                    location: ItemLocation::Inventory,
                    slot: None,
                    enchant: 0,
                    augment_id: None,
                    charges: 0,
                    created_tick: tick,
                    revision,
                };
                let delta = i64::try_from(after.count).map_err(invalid)?;
                rows.push((None, after, delta));
            }
        }
        let player = items
            .canonical
            .players
            .get_mut(&owner)
            .ok_or(RejectReason::InvalidLoad)?;
        for (_, after, _) in &rows {
            after.validate().map_err(invalid)?;
            player.inventory.insert(after.id, after.clone());
        }
        player.inventory_revision = revision;
        let (_, _, weight) = inventory_totals(&player.inventory, &rules).map_err(invalid)?;
        player.weight = weight;
        self.rebuild_v5(owner).map_err(invalid)?;
        for (before, after, count_delta) in rows {
            self.push_character_row(
                tick,
                owner,
                CharacterRow {
                    operation: ItemOperation::Harvest,
                    before,
                    after: Some(after),
                    count_delta,
                    receipt: None,
                    cause: Some(cause.to_owned()),
                    transfer: None,
                },
                events,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Family S (economy-oracle-vendor.json `s_vectors`): sow and harvest basic chances on
    /// the oracle inputs, including the negative sow basic (no floor) and the harvest floor.
    #[test]
    fn sow_and_harvest_basics_match_the_family_s_oracle() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-vendor.json"
        ))
        .unwrap();
        let s = &fixture["s_vectors"];
        let int = |v: &serde_json::Value| v.as_i64().unwrap();
        for v in s["sow"].as_array().unwrap() {
            assert_eq!(
                sow_basic(
                    int(&v["seed_level"]),
                    v["alternative"].as_bool().unwrap(),
                    int(&v["player"]),
                    int(&v["target"])
                ),
                int(&v["basic"]),
                "{v}"
            );
        }
        for v in s["harvest"].as_array().unwrap() {
            assert_eq!(
                harvest_basic(int(&v["player"]), int(&v["target"])),
                int(&v["basic"]),
                "{v}"
            );
        }
        let rules = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        let manor = &rules.parts().manor.skills;
        for v in s["harvest_count"].as_array().unwrap() {
            let skills: Vec<u32> = v["skills"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| u32::try_from(x.as_u64().unwrap()).unwrap())
                .collect();
            assert_eq!(
                harvest_count(
                    &skills,
                    &manor.strong_type_multipliers,
                    int(&v["npc_level"]),
                    int(&v["seed_level"]),
                    manor.rate_drop_manor
                ),
                v["count"].as_u64().unwrap(),
                "{v}"
            );
        }
        assert!(sow_basic(40, true, 20, 70) < 0, "no floor");
        assert_eq!(harvest_basic(10, 85), 1, "floor 1");
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::arithmetic_side_effects,
    clippy::panic
)]
mod flow_tests {
    use super::*;
    use crate::domain::zone::{CombatRole, ItemInstanceId};
    use crate::infrastructure::zone_data;

    use super::super::item_materialize::tests::{fresh_zone_and_kit_load, spawn_kit};

    const P1: u128 = 71;
    const SEED: u32 = 5016; // Dark Coda seed, castle 1, crop 5073, level 10.
    const P2_SEEDER: u128 = 99;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// A V7 zone with one kit player holding 20 seeds and one sowable pinned monster.
    fn zone() -> (ZoneState, EntityId) {
        zone_with(20)
    }

    /// The fixture with a `seeds`-count seed stack.
    fn zone_with(seeds: u64) -> (ZoneState, EntityId) {
        let (mut zone, mut load, _) = fresh_zone_and_kit_load(true);
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(id(P1), load)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let rules = std::sync::Arc::clone(&zone.phase5.as_ref().unwrap().rules);
        let pinned = rules
            .parts()
            .drops
            .npc
            .iter()
            .find(|n| n.can_be_sown && !n.is_raid)
            .unwrap()
            .id;
        let stats = zone.rules.clone().unwrap();
        let def = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
        let mut specs = crate::application::zone_bootstrap::slot_specs(&stats, &def).unwrap();
        specs.truncate(1);
        specs[0].count = 1;
        specs[0].pinned_npc_id = Some(pinned);
        zone = zone.try_with_spawn_slots(specs).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let npc = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Npc && e.ai.is_some())
            .unwrap()
            .id;
        // 20 seeds, as an earlier grant would have left them.
        let items = zone.phase4.as_mut().unwrap();
        let seed_id = items.canonical.allocator.allocate().unwrap();
        let player = items.canonical.players.get_mut(&id(P1)).unwrap();
        player.inventory_revision += 1;
        player.inventory.insert(
            seed_id,
            ItemInstance {
                creation_origin: Some(ItemCreationOrigin {
                    zone: zone.seed.zone,
                    epoch: zone.seed.epoch,
                }),
                id: seed_id,
                template_id: SEED,
                count: seeds,
                location: ItemLocation::Inventory,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: player.inventory_revision,
            },
        );
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&id(P1)).unwrap();
        player.weight = inventory_totals(&player.inventory, &rules).unwrap().2;
        (zone, npc)
    }

    fn use_input(
        zone: &ZoneState,
        key: u128,
        seq: u32,
        instance: ItemInstanceId,
        target: EntityId,
    ) -> crate::domain::zone::ZoneInput {
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &zone.entities[&id(P1)].combat.as_ref().unwrap().role
        else {
            panic!("player")
        };
        let command = crate::domain::zone::ItemCommand::new(
            id(P1),
            uuid::Uuid::from_u128(key),
            crate::domain::zone::ItemCommandKind::Use {
                instance,
                target: zone.binding(target),
            },
        )
        .unwrap()
        .with_account(player.identity.account_id)
        .unwrap();
        crate::domain::zone::ZoneInput::session(
            id(P1),
            crate::domain::zone::SessionGeneration(1),
            seq,
            crate::domain::zone::ZoneCommand::Item(Box::new(command)),
        )
    }

    /// One tick, then the snapshot restores (the item-cast lanes are validated there).
    fn step(
        zone: &mut ZoneState,
        inputs: Vec<crate::domain::zone::ZoneInput>,
    ) -> crate::domain::zone::AppliedTick {
        let tick = zone.run_tick(zone.draft(inputs)).unwrap();
        let items = std::sync::Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let economy = std::sync::Arc::clone(&zone.phase5.as_ref().unwrap().rules);
        let restored =
            ZoneState::from_snapshot_with_bundles(zone.snapshot(), Some(items), Some(economy))
                .unwrap_or_else(|e| panic!("tick {}: restore {e:?}", tick.tick.0));
        assert_eq!(restored.state_digest(), zone.state_digest());
        tick
    }

    fn seed_instance(zone: &ZoneState, template: u32) -> Option<ItemInstanceId> {
        zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .values()
            .find(|i| i.template_id == template)
            .map(|i| i.id)
    }

    /// Run ticks until `skill`'s cast result for P1 lands; the result's outcome per target.
    fn until_result(zone: &mut ZoneState, skill: u32) -> Vec<SkillOutcome> {
        for _ in 0..80 {
            let tick = step(zone, Vec::new());
            for event in &tick.events {
                if let ZoneEvent::SkillCastResult {
                    caster,
                    skill: s,
                    targets,
                    ..
                } = event
                {
                    if caster.entity == id(P1) && s.id.0 == skill {
                        return targets.iter().map(|t| t.outcome).collect();
                    }
                }
            }
        }
        panic!("no {skill} result in 80 ticks");
    }

    /// The `UseItem` -> cast -> impact path end to end (coordinator ask for 3.6): a
    /// count-1 seed stack is consumed at the Sow impact while its item-cast lane is still
    /// open, every tick restores (lane validation), and the Harvester cast harvests.
    #[test]
    fn seed_and_harvester_run_through_use_item_cast_and_impact() {
        let (mut zone, npc) = zone_with(1);
        // Next to the player, in its AOI.
        let here = zone.entities[&id(P1)].pos;
        let from = zone.entities[&npc].pos;
        let body = zone.entities.get_mut(&npc).unwrap();
        body.pos = here;
        body.ai.as_mut().unwrap().home = here;
        zone.aoi.relocate(npc, from, here);
        let mut key = 500;
        let mut seq = 1;
        let mut sown = false;
        for _ in 0..6 {
            let seed = seed_instance(&zone, SEED).unwrap_or_else(|| {
                // Another 1-stack for the retry.
                let items = zone.phase4.as_mut().unwrap();
                let new = items.canonical.allocator.allocate().unwrap();
                let player = items.canonical.players.get_mut(&id(P1)).unwrap();
                let mut item = player.inventory.values().next().unwrap().clone();
                item.id = new;
                item.template_id = SEED;
                item.count = 1;
                item.location = ItemLocation::Inventory;
                item.slot = None;
                item.enchant = 0;
                item.augment_id = None;
                item.charges = 0;
                player.inventory.insert(new, item);
                let rules = std::sync::Arc::clone(&items.rules);
                let player = items.canonical.players.get_mut(&id(P1)).unwrap();
                player.weight = inventory_totals(&player.inventory, &rules).unwrap().2;
                new
            });
            let input = use_input(&zone, key, seq, seed, npc);
            let tick = step(&mut zone, vec![input]);
            key += 1;
            seq += 1;
            assert!(tick.dispositions.is_empty(), "use refused: {:?}", tick.dispositions);
            assert_eq!(
                zone.phase5.as_ref().unwrap().canonical.npcs[&npc]
                    .seed
                    .map(|b| b.seeder),
                Some(id(P1)),
                "bound once the Sowing cast started"
            );
            let outcomes = until_result(&mut zone, 2097);
            assert_eq!(seed_instance(&zone, SEED), None, "the last seed is consumed at impact");
            assert!(
                zone.phase4
                    .as_ref()
                    .unwrap()
                    .canonical
                    .item_casts
                    .is_empty(),
                "lane closed"
            );
            if outcomes == vec![SkillOutcome::Hit] {
                sown = true;
                break;
            }
            // Reuse 10 s.
            for _ in 0..101 {
                step(&mut zone, Vec::new());
            }
        }
        assert!(sown, "sown in 6 tries at basic 70");
        let (crop, count) = zone.phase5.as_ref().unwrap().canonical.npcs[&npc]
            .harvest
            .unwrap();
        // Kill it through real combat (the engine death path; seeded: corpse 7 + 10 s),
        // then harvest through the Harvester.
        zone.entities
            .get_mut(&npc)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .hp = 1;
        let session = |seq: u32, command| {
            crate::domain::zone::ZoneInput::session(
                id(P1),
                crate::domain::zone::SessionGeneration(1),
                seq,
                command,
            )
        };
        let attack = step(
            &mut zone,
            vec![
                session(
                    seq,
                    crate::domain::zone::ZoneCommand::SetTarget {
                        entity: id(P1),
                        target: Some(npc),
                    },
                ),
                session(seq + 1, crate::domain::zone::ZoneCommand::Attack { entity: id(P1) }),
            ],
        );
        assert!(attack.dispositions.is_empty(), "{:?}", attack.dispositions);
        seq += 2;
        for _ in 0..200 {
            if zone.entities[&npc].targeting.dead {
                break;
            }
            step(&mut zone, Vec::new());
        }
        assert!(zone.entities[&npc].targeting.dead, "killed");
        assert!(zone.phase5.as_ref().unwrap().canonical.npcs[&npc].seeded);
        let items = zone.phase4.as_mut().unwrap();
        let harvester = items.canonical.allocator.allocate().unwrap();
        let player = items.canonical.players.get_mut(&id(P1)).unwrap();
        let mut item = player.inventory.values().next().unwrap().clone();
        item.id = harvester;
        item.template_id = 5125;
        item.count = 1;
        item.location = ItemLocation::Inventory;
        item.slot = None;
        item.enchant = 0;
        item.augment_id = None;
        item.charges = 0;
        player.inventory.insert(harvester, item);
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&id(P1)).unwrap();
        player.weight = inventory_totals(&player.inventory, &rules).unwrap().2;
        let input = use_input(&zone, key, seq, harvester, npc);
        let tick = step(&mut zone, vec![input]);
        assert!(tick.dispositions.is_empty(), "harvester refused: {:?}", tick.dispositions);
        let outcomes = until_result(&mut zone, 2098);
        assert_eq!(outcomes, vec![SkillOutcome::Hit], "basic 100 at equal levels");
        assert_eq!(held(&zone, crop), count);
        assert_eq!(held(&zone, 5125), 1, "the harvester is not consumed");
        assert!(zone
            .phase4
            .as_ref()
            .unwrap()
            .canonical
            .item_casts
            .is_empty());
    }

    fn held(zone: &ZoneState, template: u32) -> u64 {
        zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .values()
            .filter(|i| i.template_id == template)
            .map(|i| i.count)
            .sum()
    }

    /// The V7 item view admits seeds and the harvester (handler + skill cleared); the V6
    /// bundle keeps them deferred.
    #[test]
    fn the_v7_item_view_admits_seeds_and_the_harvester() {
        let rules = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        for template in [SEED, 5125] {
            let v7 = &rules.v7_items().unwrap().parts().capabilities[&template];
            assert!(v7.admitted && v7.use_blockers.is_empty(), "{template}: {v7:?}");
            assert!(bundle
                .items
                .parts()
                .capabilities
                .get(&template)
                .is_none_or(|c| !c.admitted));
        }
        // A herb stays deferred (its skill is not promoted).
        assert!(!rules.v7_items().unwrap().parts().capabilities[&8600].admitted);
    }

    /// §10.6 end to end on the handlers and effects: Seed binds once; each Sow consumes one
    /// seed before its draw; success seeds the NPC with its crop; only the seeder harvests a
    /// seeded corpse, which grants a fresh crop with a `harvest` row.
    #[test]
    fn seed_sow_and_harvest_follow_the_pinned_order() {
        let (mut zone, npc) = zone();
        zone.seed_use_v7(id(P1), SEED, Some(npc)).unwrap();
        // Another castle's seed is refused; an unsown binding is replaced (pinned).
        assert_eq!(zone.seed_use_v7(id(P1), 5024, Some(npc)), Err(RejectReason::NotSeedable));
        zone.seed_bind_v7(id(P2_SEEDER), SEED, npc);
        zone.seed_use_v7(id(P1), SEED, Some(npc)).unwrap();
        zone.seed_bind_v7(id(P1), SEED, npc);
        let caster = zone.binding(id(P1)).unwrap();
        let target = zone.binding(npc).unwrap();
        let tick = zone.next_tick;
        let mut outcome = SkillOutcome::Resist;
        let mut sown = 0;
        while outcome != SkillOutcome::Hit && sown < 20 {
            let mut events = Vec::new();
            let before = held(&zone, SEED);
            outcome = zone
                .sow_v7(tick, caster, target, CastId(5), &mut events)
                .unwrap();
            assert_eq!(held(&zone, SEED), before - 1, "consumed before the roll");
            sown += 1;
        }
        assert_eq!(outcome, SkillOutcome::Hit, "sown within 20 seeds");
        let sidecar = &zone.phase5.as_ref().unwrap().canonical.npcs[&npc];
        assert!(sidecar.seeded);
        let (crop, count) = sidecar.harvest.unwrap();
        assert_eq!(crop, 5073);
        // Seeded: the Seed handler refuses, and a second Sow fails its guard without
        // consuming a seed.
        assert_eq!(zone.seed_use_v7(id(P1), SEED, Some(npc)), Err(RejectReason::NotSeedable));
        let before = held(&zone, SEED);
        let mut events = Vec::new();
        assert_eq!(
            zone.sow_v7(tick, caster, target, CastId(6), &mut events)
                .unwrap(),
            SkillOutcome::Miss
        );
        assert_eq!(held(&zone, SEED), before);
        // Harvest needs a dead target, and only the seeder.
        assert_eq!(zone.harvester_use_v7(Some(npc)), Err(RejectReason::HarvestNotAllowed));
        zone.entities.get_mut(&npc).unwrap().targeting.dead = true;
        zone.harvester_use_v7(Some(npc)).unwrap();
        let mut outcome = SkillOutcome::Resist;
        let mut events = Vec::new();
        for _ in 0..20 {
            outcome = zone
                .harvesting_v7(tick, caster, target, CastId(7), &mut events)
                .unwrap();
            if outcome == SkillOutcome::Hit {
                break;
            }
        }
        assert_eq!(outcome, SkillOutcome::Hit);
        assert_eq!(held(&zone, 5073), count);
        assert!(zone.phase5.as_ref().unwrap().canonical.npcs[&npc]
            .harvest
            .is_none());
        let rows: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == id(P1) => Some(p.ledger.clone()),
                _ => None,
            })
            .flatten()
            .filter(|r| r.operation == ItemOperation::Harvest)
            .collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].before.is_none(), "a fresh crop");
    }
}
