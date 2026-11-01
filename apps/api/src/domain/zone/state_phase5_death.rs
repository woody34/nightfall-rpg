//! V7 death resolution and the ground lifecycle (design §1.2–§1.3, §2.1 phase 2, §2.2, §3.5–
//! §3.8): the drop actor (X7), the corpse then death lists (`drops.rs`), NPC loot on the ground
//! with its P1 identity, both protections and the 5 s-grid decay, the V7 corpse time, and the
//! NPC economy sidecar lifecycle. Every hook runs only when the zone has a Phase 5 block.

use std::sync::Arc;

use crate::domain::zone::drops::{roll_list, DropDraws, DropError, DropVictim};
use crate::domain::zone::phase5::{DropProtect, LootOwner, NpcEconomy};
use crate::domain::zone::phase5_event::{GroundItemDetail, GroundProtectionView};
use crate::domain::zone::phase5_rules::{DropScope, Phase5Rules};
use crate::domain::zone::{
    EntityId, EntityKind, Fixed, GroundItem, GroundSource, ItemCreationOrigin, ItemInstance,
    ItemLocation, SessionGeneration, Speed, Tick, TickError, Vec2Fixed, ZoneEvent, ZoneState,
};

use super::Spawned;

/// `PartyRange` 1,600 L2 units in milli-tiles, `m(1600) = trunc0(1600 · 1000 / 32)` (X2).
const PARTY_RANGE_MILLI: i64 = 50_000;
/// `RANDOM_ITEM_DROP_LIMIT` (`L2Npc.java`, NPC loot offsets `[−70, 70]`).
const NPC_DROP_OFFSET: i32 = 70;
/// The ground-decay scanner grid (X8): `ItemsAutoDestroy` runs every 5,000 ms = 50 ticks.
const DECAY_GRID_TICKS: u64 = 50;
/// `DefaultCorpseTime` (`GC/npc.properties:86`) for an NPC without a pinned row.
const DEFAULT_CORPSE_TIME: u64 = 7;
/// `SpoiledCorpseExtendTime = 10` s in ticks (`GC/npc.properties:90`).
const SPOILED_CORPSE_EXTEND_TICKS: u64 = 100;

impl DropDraws for ZoneState {
    fn draw_pair(&mut self) -> u64 {
        self.draw_u64()
    }

    fn draw_below(&mut self, n: u32) -> u32 {
        self.roll_below(n)
    }
}

/// `m(L) = trunc0(L · 1000 / 32)`: L2 units to milli-tiles (X2).
fn milli(l2_units: i32) -> i32 {
    l2_units.saturating_mul(1000) / 32
}

impl ZoneState {
    /// Latch a V7 invariant failure from an infallible caller (`kill()`); the tick driver
    /// turns it into [`TickError::Phase5Invariant`] before anything is flushed.
    pub(super) fn latch_phase5_fault(&mut self, error: &DropError) {
        if let Some(phase5) = &mut self.phase5 {
            if phase5.fault.is_none() {
                phase5.fault = Some(error.to_string());
            }
        }
    }

    /// Fail the tick if a V7 hook latched an invariant failure.
    pub(super) fn take_phase5_fault(&mut self) -> Result<(), TickError> {
        match self.phase5.as_mut().and_then(|p| p.fault.take()) {
            Some(_) => Err(TickError::Phase5Invariant),
            None => Ok(()),
        }
    }

    /// The pinned `drops.toml` row bound to an AI NPC through its spawn slot (story 3.3).
    pub(super) fn pinned_npc(&self, npc: EntityId) -> Option<u32> {
        let slot = self.entities.get(&npc)?.ai.as_ref()?.slot;
        self.slots.get(usize::from(slot.slot))?.pinned_npc_id
    }

    /// The drop actor of design §3.6 (X7): over the hate ledger in ascending `EntityId`, player
    /// rows with `damage > 1` within `PartyRange` (`4·dist² ≤ (2·50,000 + npc_radius)²`); the
    /// first strict maximum wins. If there is none, or it is not online (an admitted player
    /// entity in this zone), the killing blow's attacker when it is a player. A row whose
    /// player has left the zone cannot be range-checked and does not count.
    fn drop_actor(&self, victim: EntityId, attacker: Option<EntityId>) -> Option<EntityId> {
        let npc = self.entities.get(&victim)?;
        let radius = npc
            .combat
            .as_ref()
            .map_or(0, |c| i64::from(c.collision_radius.raw()));
        let reach = PARTY_RANGE_MILLI.saturating_mul(2).saturating_add(radius);
        let bound = square(reach);
        let mut best: Option<(EntityId, u64)> = None;
        if let Some(ledger) = self.hate.get(&victim) {
            for (id, row) in ledger.iter() {
                if row.damage <= 1 {
                    continue;
                }
                let Some(player) = self
                    .entities
                    .get(&id)
                    .filter(|e| e.kind == EntityKind::Player)
                else {
                    continue;
                };
                let dx = i64::from(player.pos.x.raw()).saturating_sub(i64::from(npc.pos.x.raw()));
                let dy = i64::from(player.pos.y.raw()).saturating_sub(i64::from(npc.pos.y.raw()));
                let dist = square(dx).saturating_add(square(dy));
                if dist.saturating_mul(4) > bound {
                    continue;
                }
                if best.is_none_or(|(_, damage)| row.damage > damage) {
                    best = Some((id, row.damage));
                }
            }
        }
        best.map(|(id, _)| id).or_else(|| {
            attacker.filter(|id| {
                self.entities
                    .get(id)
                    .is_some_and(|e| e.kind == EntityKind::Player)
            })
        })
    }

    /// The reward snapshot `R` of design §4.5 (the §3.6 filter: player rows with `damage > 1`
    /// within `PartyRange`), the facts of every player it and its parties name, and the pass.
    fn reward_awards(
        &self,
        victim: EntityId,
        rules: &Phase5Rules,
        npc: &crate::domain::zone::phase5_rules::RewardNpc,
    ) -> Result<Vec<crate::domain::zone::rewards::Award>, DropError> {
        use crate::domain::zone::rewards::{base_rewards, reward_pass, RewardPlayer};
        use std::collections::BTreeMap;
        let Some(entity) = self.entities.get(&victim) else {
            return Ok(Vec::new());
        };
        let npc_level = entity
            .combat
            .as_ref()
            .map(|c| c.sheet.level())
            .ok_or(DropError::Rules("victim without combat"))?;
        let radius = entity
            .combat
            .as_ref()
            .map_or(0, |c| i64::from(c.collision_radius.raw()));
        let bound = square(PARTY_RANGE_MILLI.saturating_mul(2).saturating_add(radius));
        let in_range = |pos: Vec2Fixed| {
            let dx = i64::from(pos.x.raw()).saturating_sub(i64::from(entity.pos.x.raw()));
            let dy = i64::from(pos.y.raw()).saturating_sub(i64::from(entity.pos.y.raw()));
            square(dx).saturating_add(square(dy)).saturating_mul(4) <= bound
        };
        let facts = |id: EntityId| -> Option<RewardPlayer> {
            let player = self
                .entities
                .get(&id)
                .filter(|e| e.kind == EntityKind::Player)?;
            Some(RewardPlayer {
                level: player.combat.as_ref().map_or(1, |c| c.sheet.level()),
                alive: !player.targeting.dead,
                in_range: in_range(player.pos),
            })
        };
        let mut damage = BTreeMap::new();
        if let Some(ledger) = self.hate.get(&victim) {
            for (id, row) in ledger.iter() {
                if row.damage > 1 && facts(id).is_some_and(|f| f.in_range) {
                    damage.insert(id, row.damage);
                }
            }
        }
        if damage.is_empty() {
            return Ok(Vec::new());
        }
        let block = self
            .phase5
            .as_ref()
            .map(|p| &p.canonical)
            .ok_or(DropError::Rules("V7 block vanished"))?;
        let mut parties = Vec::new();
        let mut players = BTreeMap::new();
        for id in damage.keys() {
            let party = block
                .players
                .get(id)
                .and_then(|p| p.party)
                .and_then(|party| block.parties.get(&party));
            if let Some(party) = party {
                if !parties.contains(&party.members) {
                    parties.push(party.members.clone());
                }
            }
        }
        for id in damage.keys().chain(parties.iter().flatten()) {
            // A member that left the zone is not alive here (departure removes it, §4.6).
            let f = facts(*id).unwrap_or(RewardPlayer {
                level: 1,
                alive: false,
                in_range: false,
            });
            players.insert(*id, f);
        }
        let base = base_rewards(rules, npc_level, &npc.exp_rate, &npc.sp)?;
        reward_pass(rules, &damage, &parties, &players, npc_level, base)
    }

    /// Apply the stashed B16 awards of this death (`kill()` step 7): the existing XP path (level
    /// ups unchanged) and the V5 SP credit. `false` when this death has none (V5/V6, or no
    /// pinned row), so the caller keeps the killing-blow award.
    pub(super) fn apply_phase5_rewards(
        &mut self,
        tick: Tick,
        victim: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) -> bool {
        let Some((owner, awards)) = self.phase5.as_mut().and_then(|p| p.rewards.take()) else {
            return false;
        };
        if owner != victim {
            return false;
        }
        for award in awards {
            if award.exp > 0 {
                self.award_xp(tick, award.player, award.exp, events);
            }
            if award.sp > 0 {
                self.credit_sp_amount(tick, award.player, award.sp, events);
            }
        }
        true
    }

    /// Credit SP exactly as the V5 skill-kill credit does (`SpGained` with the live
    /// incarnation), for an amount computed by B16.
    fn credit_sp_amount(
        &mut self,
        tick: Tick,
        player: EntityId,
        amount: u64,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(c) = self
            .entities
            .get_mut(&player)
            .and_then(|e| e.combat.as_mut())
        else {
            return;
        };
        let incarnation = c.incarnation;
        if let crate::domain::zone::CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        {
            // `PlayableStat.addSp` (`:165-181`): SP is an `int`; at `Integer.MAX_VALUE`
            // nothing is added, above it the total saturates.
            const SP_MAX: u64 = 2_147_483_647;
            let old = p.class_state.sp;
            if old >= SP_MAX || amount == 0 {
                return;
            }
            p.class_state.sp = old.saturating_add(amount).min(SP_MAX);
            events.push(ZoneEvent::SpGained {
                tick,
                entity: player,
                incarnation,
                amount: p.class_state.sp.saturating_sub(old),
                total: p.class_state.sp,
            });
        }
    }

    /// The V7 `kill()` hook (design §2.2), between `EntityDied` and `npc_died`: an NPC rolls
    /// its corpse list (if spoiled) into the sweep holders, then its death list, and creates
    /// the loot on the ground. A player victim runs the PK loop (story 3.3, PK commit).
    pub(super) fn death_v7(
        &mut self,
        tick: Tick,
        victim: EntityId,
        attacker: Option<EntityId>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), DropError> {
        let Some(phase5) = &self.phase5 else {
            return Ok(());
        };
        let rules = Arc::clone(&phase5.rules);
        let Some(entity) = self.entities.get(&victim) else {
            return Ok(());
        };
        if entity.kind != EntityKind::Npc {
            return Ok(());
        }
        let Some(pinned) = self.pinned_npc(victim) else {
            return Ok(());
        };
        // P5-D12: every pinned template has a rewards.toml row (the only reward source).
        let reward = rules
            .reward_npc(pinned)
            .ok_or(DropError::Rules("pinned npc without a rewards.toml row"))?;
        // B16 (design §4.5): the awards come from the reward snapshot taken now, before the
        // hate ledger is removed; they are applied at kill() step 7. Draw-free.
        let awards = self.reward_awards(victim, &rules, reward)?;
        let raid = reward.is_raid;
        if let Some(phase5) = &mut self.phase5 {
            phase5.rewards = Some((victim, awards));
        }
        // An NPC without a `<death>`/`<corpse>` list has no drops.toml row: nothing to roll.
        let Some(npc) = rules.drop_npc(pinned) else {
            return Ok(());
        };
        let Some(actor) = self.drop_actor(victim, attacker) else {
            return Ok(());
        };
        let level = |id: EntityId| {
            self.entities
                .get(&id)
                .and_then(|e| e.combat.as_ref())
                .map(|c| i64::from(c.sheet.level()))
        };
        let victim_facts = DropVictim {
            raid,
            level: level(victim).ok_or(DropError::Rules("victim without combat"))?,
        };
        let killer_level = level(actor).ok_or(DropError::Rules("drop actor without combat"))?;
        let origin = entity.pos;
        let spoiled = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.npcs.get(&victim))
            .is_some_and(|n| n.spoiler.is_some());
        if spoiled {
            let sweep =
                roll_list(&rules, npc, DropScope::Corpse, victim_facts, killer_level, self)?;
            if let Some(sidecar) = self
                .phase5
                .as_mut()
                .and_then(|p| p.canonical.npcs.get_mut(&victim))
            {
                sidecar.sweep = (!sweep.is_empty()).then_some(sweep);
            }
        }
        // §5.3: `CorpseStateChanged` at death when the corpse is spoiled or seeded.
        if spoiled
            || self
                .phase5
                .as_ref()
                .and_then(|p| p.canonical.npcs.get(&victim))
                .is_some_and(|n| n.seeded)
        {
            self.corpse_state_v7(tick, victim, false, events);
        }
        let holders = roll_list(&rules, npc, DropScope::Death, victim_facts, killer_level, self)?;
        for holder in holders {
            self.spawn_npc_loot(tick, victim, origin, actor, raid, holder, &rules, events)?;
        }
        Ok(())
    }

    /// `L2Npc.dropItem` (design §3.5): per created instance (once if stackable,
    /// `MultipleItemDrop = True` otherwise), the two position draws, one allocated instance,
    /// the P6 position fallback, both protections, the decay deadline and the spawn.
    #[allow(clippy::too_many_arguments)] // The pinned call's inputs, explicit.
    fn spawn_npc_loot(
        &mut self,
        tick: Tick,
        npc: EntityId,
        origin: Vec2Fixed,
        actor: EntityId,
        raid: bool,
        (template, count): (u32, u64),
        rules: &Phase5Rules,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), DropError> {
        let stackable = self
            .phase4
            .as_ref()
            .and_then(|items| items.rules.parts().items.get(&template))
            // D10: a V7 epoch runs on the V7 item view (V6 ∪ closure), so every catalogue
            // template resolves; a missing definition is an invariant failure.
            .ok_or(DropError::Rules("drop template without an item definition"))?
            .stackable;
        let instances = if stackable { 1 } else { count };
        let ground = &rules.parts().ground.ground;
        for _ in 0..instances {
            let dx = i32::try_from(self.roll_below(141))
                .unwrap_or(0)
                .saturating_sub(NPC_DROP_OFFSET);
            let dy = i32::try_from(self.roll_below(141))
                .unwrap_or(0)
                .saturating_sub(NPC_DROP_OFFSET);
            let id = self
                .phase4
                .as_mut()
                .ok_or(DropError::Rules("V7 without the V6 block"))?
                .canonical
                .allocator
                .allocate()
                .map_err(|_| DropError::Rules("item allocator exhausted"))?;
            let shifted = Vec2Fixed::new(
                Fixed::from_raw(origin.x.raw().saturating_add(milli(dx))),
                Fixed::from_raw(origin.y.raw().saturating_add(milli(dy))),
            );
            // P6: an out-of-bounds point falls back to the source position (draws consumed).
            let pos = if self.bounds.contains(shifted) {
                shifted
            } else {
                origin
            };
            let item = ItemInstance {
                creation_origin: Some(ItemCreationOrigin {
                    zone: self.seed.zone,
                    epoch: self.seed.epoch,
                }),
                id,
                template_id: template,
                count: if stackable { count } else { 1 },
                location: ItemLocation::Ground,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: tick,
                revision: 0,
            };
            item.validate()
                .map_err(|_| DropError::Rules("invalid ground item"))?;
            let entity = GroundItem::entity_id(self.seed.zone, self.seed.epoch, id, tick);
            let ticks = |ms: u64| Tick(tick.0.saturating_add(ms / 100));
            let record = GroundItem {
                entity,
                item,
                pos,
                dropped_tick: tick,
                source: GroundSource::NpcLoot { npc },
                // ItemTable "Loot": non-raid with AutoLoot = False → the actor for 15 s; raids
                // get the first-CC leader only, and command channels are vacuous before Phase 7.
                loot_owner: (!raid && !ground.auto_loot).then(|| LootOwner {
                    owner: actor,
                    until: ticks(ground.loot_owner_ms),
                }),
                drop_protection: Some(DropProtect {
                    owner: actor,
                    until: ticks(ground.drop_protection_ms),
                }),
                decay_due: decay_due(rules, template, tick),
            };
            let detail = ground_detail(rules, &record);
            let mut spawned = self
                .spawn(
                    tick,
                    Spawned {
                        id: entity,
                        kind: EntityKind::GroundItem,
                        name: "",
                        pos,
                        speed: Speed::from_milli_tiles_per_tick(0),
                        generation: SessionGeneration::default(),
                        combat: None,
                    },
                )
                // P1: a collision with a live entity is corruption, never a retry.
                .map_err(|_| DropError::Rules("ground entity collision"))?;
            for event in &mut spawned {
                if let ZoneEvent::EntitySpawn { phase5, .. } = event {
                    *phase5 = Some(Box::new(detail.clone()));
                }
            }
            // §11.3 "NPC loot creation": the ground owner's fresh `loot` row.
            self.ledger_loot(&record, &format!("{}@{}", npc.as_uuid(), tick.0));
            self.phase5
                .as_mut()
                .ok_or(DropError::Rules("V7 block vanished"))?
                .canonical
                .ground
                .insert(entity, record);
            events.extend(spawned);
        }
        Ok(())
    }

    /// The V7 corpse time (design §1.3): `10 × corpseTime + (100 iff spoiled or seeded)`, with
    /// the pinned per-NPC `corpseTime` or `DefaultCorpseTime = 7`. `None` outside V7.
    pub(super) fn v7_corpse_ticks(&self, npc: EntityId) -> Option<u64> {
        let phase5 = self.phase5.as_ref()?;
        let seconds = self
            .pinned_npc(npc)
            .and_then(|pinned| phase5.rules.reward_npc(pinned))
            .map_or(DEFAULT_CORPSE_TIME, |row| row.corpse_time);
        let extended = phase5
            .canonical
            .npcs
            .get(&npc)
            .is_some_and(|n| n.spoiler.is_some() || n.seeded);
        Some(seconds.saturating_mul(10).saturating_add(if extended {
            SPOILED_CORPSE_EXTEND_TICKS
        } else {
            0
        }))
    }

    /// Phase 2 V7 expiry (design §2.1): (a) protection deadlines reached are cleared, (b) on
    /// the 50-tick grid every ground item with `decay_due ≤ tick` despawns, in `EntityId`
    /// order. (c)–(f) — requests, loot votes, invites, restocks — arrive with their stories.
    pub(super) fn expire_phase5(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        // §6.4: due limited stock refills in phase 2.
        self.restock_v7(tick);
        let Some(phase5) = &mut self.phase5 else {
            return;
        };
        for ground in phase5.canonical.ground.values_mut() {
            if ground.loot_owner.is_some_and(|o| o.until <= tick) {
                ground.loot_owner = None;
            }
            if ground.drop_protection.is_some_and(|p| p.until <= tick) {
                ground.drop_protection = None;
            }
        }
        if !tick.0.is_multiple_of(DECAY_GRID_TICKS) {
            return;
        }
        let due: Vec<EntityId> = phase5
            .canonical
            .ground
            .values()
            .filter(|g| g.decay_due.is_some_and(|due| due <= tick))
            .map(|g| g.entity)
            .collect();
        for entity in due {
            let record = self
                .phase5
                .as_mut()
                .and_then(|phase5| phase5.canonical.ground.remove(&entity));
            if let Some(record) = record {
                self.ledger_decay(tick, &record);
            }
            events.extend(self.despawn(tick, entity));
        }
    }

    /// The NPC economy sidecar exists from spawn and is cleared at respawn (design §1.3,
    /// `L2Attackable.onSpawn`): a fresh default per (re)spawned AI NPC.
    pub(super) fn phase5_npc_spawned(&mut self, entity: EntityId) {
        let npc = self
            .entities
            .get(&entity)
            .is_some_and(|e| e.kind == EntityKind::Npc && e.ai.is_some());
        if let (true, Some(phase5)) = (npc, &mut self.phase5) {
            phase5.canonical.npcs.insert(entity, NpcEconomy::default());
        }
    }

    /// A despawned NPC's sidecar goes with it; any untaken sweep holders vanish (no instances).
    pub(super) fn phase5_npc_despawned(&mut self, entity: EntityId) {
        if let Some(phase5) = &mut self.phase5 {
            phase5.canonical.npcs.remove(&entity);
        }
    }

    /// `GroundItemDetail` for an observer's spawn of a ground entity (design §1.13).
    pub(super) fn ground_spawn_detail(&self, entity: EntityId) -> Option<GroundItemDetail> {
        let phase5 = self.phase5.as_ref()?;
        let ground = phase5.canonical.ground.get(&entity)?;
        Some(ground_detail(&phase5.rules, ground))
    }
}

/// `x²` for the range tests, saturating (coordinates are `i32` milli-tiles, so `x²` < 2^63).
fn square(x: i64) -> i64 {
    x.saturating_mul(x)
}

/// The decay deadline of NPC loot (design §3.8, X8): the template's `auto_destroy_time`, else
/// the herb time, else the general decay; `decay_due` is the first 50-tick grid point strictly
/// after `dropped + limit`. `ProtectedItems` is empty.
fn decay_due(rules: &Phase5Rules, template: u32, dropped: Tick) -> Option<Tick> {
    let ground = &rules.parts().ground.ground;
    let seconds = ground
        .auto_destroy_time
        .iter()
        .find(|row| row.item_id == template)
        .map_or(
            if rules.is_herb(template) {
                ground.herb_decay_seconds
            } else {
                ground.decay_seconds
            },
            |row| row.seconds,
        );
    if seconds == 0 {
        return None;
    }
    let limit = seconds.saturating_mul(10);
    let last = dropped.0.saturating_add(limit);
    Some(Tick(
        (last / DECAY_GRID_TICKS)
            .saturating_add(1)
            .saturating_mul(DECAY_GRID_TICKS),
    ))
}

fn ground_detail(rules: &Phase5Rules, ground: &GroundItem) -> GroundItemDetail {
    GroundItemDetail {
        item_instance_id: ground.item.id,
        template_id: ground.item.template_id,
        count: ground.item.count,
        enchant: ground.item.enchant,
        loot_owner: ground.loot_owner.map(|o| GroundProtectionView {
            owner: o.owner,
            until: o.until,
        }),
        drop_protection: ground.drop_protection.map(|p| GroundProtectionView {
            owner: p.owner,
            until: p.until,
        }),
        dropped_tick: ground.dropped_tick,
        herb: rules.is_herb(ground.item.template_id),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm
)]
mod tests {
    use super::*;

    use crate::domain::zone::phase5_rules::DropNode;
    use crate::domain::zone::{ItemInstanceRange, PlayerLoad, ZoneBounds, ZoneId, ZoneSeed};
    use crate::infrastructure::{class_data, economy_data, item_data, rules_data, zone_data};

    fn v7_zone() -> ZoneState {
        let bundle = item_data::embedded_phase4_bundle().unwrap();
        ZoneState::new_phase5(
            ZoneSeed {
                zone: ZoneId(75),
                epoch: 5,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            rules_data::load_rules(&rules_data::RulesSource::embedded())
                .unwrap()
                .rules,
            class_data::load_classes(&class_data::ClassSource::embedded())
                .unwrap()
                .registry,
            (*bundle.skills).clone(),
            bundle.items.clone(),
            ItemInstanceRange::new(1, 1, 10_000).unwrap(),
            economy_data::embedded_phase5_rules().unwrap(),
        )
        .unwrap()
    }

    /// A non-raid pinned npc whose every drop template has a V6 item definition (D10).
    fn v6_only_npc(rules: &Phase5Rules, items: &crate::domain::zone::Phase4Rules) -> u32 {
        let known = |nodes: &[DropNode]| {
            nodes.iter().all(|node| match node {
                DropNode::Item { id, .. } => items.parts().items.contains_key(id),
                DropNode::Group { items: rows, .. } => rows
                    .iter()
                    .all(|row| items.parts().items.contains_key(&row.id)),
            })
        };
        rules
            .parts()
            .drops
            .npc
            .iter()
            .find(|npc| {
                !npc.is_raid && !npc.death.is_empty() && known(&npc.death) && known(&npc.corpse)
            })
            .unwrap()
            .id
    }

    /// z-review-3 P2-1: an unknown `pinned_npc_id` refuses slot install and restore on a V7
    /// zone (a pinned template is one with a `rewards.toml` row), instead of halting the
    /// zone at that NPC's first kill.
    #[test]
    fn unknown_pinned_npc_ids_refuse_install_and_restore() {
        let zone = v7_zone();
        let stats = zone.rules.clone().unwrap();
        let def = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
        let mut specs = crate::application::zone_bootstrap::slot_specs(&stats, &def).unwrap();
        specs.truncate(1);
        specs[0].pinned_npc_id = Some(999_999);
        let error = zone
            .clone()
            .try_with_spawn_slots(specs.clone())
            .unwrap_err();
        assert!(format!("{error:?}").contains("999999"), "{error:?}");
        // An NPC without a drop list is still a pinned template (rewards only).
        specs[0].pinned_npc_id = Some(102);
        let good = zone.clone().try_with_spawn_slots(specs.clone()).unwrap();
        let mut snapshot = good.snapshot();
        let items = Arc::clone(&good.phase4.as_ref().unwrap().rules);
        let economy = Arc::clone(&good.phase5.as_ref().unwrap().rules);
        ZoneState::from_snapshot_with_bundles(
            snapshot.clone(),
            Some(Arc::clone(&items)),
            Some(Arc::clone(&economy)),
        )
        .unwrap();
        snapshot.spawn_slots[0].pinned_npc_id = Some(999_999);
        assert!(
            ZoneState::from_snapshot_with_bundles(snapshot, Some(items), Some(economy)).is_err()
        );
        // Older epochs never carry the binding, so nothing is checked there.
        let legacy = ZoneState::new(
            crate::domain::zone::ZoneSeed {
                zone: crate::domain::zone::ZoneId(75),
                epoch: 5,
            },
            crate::domain::zone::ZoneBounds::new(
                Vec2Fixed::from_tiles(0, 0),
                Vec2Fixed::from_tiles(256, 256),
            )
            .unwrap(),
            0,
        )
        .with_rules(stats);
        specs[0].pinned_npc_id = Some(999_999);
        let legacy = legacy.try_with_spawn_slots(specs).unwrap();
        assert!(legacy.spawn_slots()[0].pinned_npc_id.is_none());
    }

    /// Story 3.3 end to end: a V7 kill rolls the pinned lists, puts the loot on the ground with
    /// P1 ids, protections and decay, extends nothing (not spoiled), sets the V7 corpse time,
    /// keeps the block valid, and the decay grid removes the loot.
    #[test]
    fn a_v7_kill_drops_pinned_loot_on_the_ground_and_decay_removes_it() {
        let mut zone = v7_zone();
        let rules = Arc::clone(&zone.phase5.as_ref().unwrap().rules);
        let items = Arc::clone(&zone.phase4.as_ref().unwrap().rules);
        let pinned = v6_only_npc(&rules, &items);
        let stats = zone.rules.clone().unwrap();
        let def = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
        let mut specs = crate::application::zone_bootstrap::slot_specs(&stats, &def).unwrap();
        specs.truncate(1);
        specs[0].count = 1;
        specs[0].pinned_npc_id = Some(pinned);
        zone = zone.with_spawn_slots(specs);
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let npc = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Npc && e.ai.is_some())
            .unwrap()
            .id;
        assert!(
            zone.phase5
                .as_ref()
                .unwrap()
                .canonical
                .npcs
                .contains_key(&npc),
            "sidecar from spawn"
        );
        // A player next to it with the top damage row.
        let player = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        let npc_pos = zone.entities[&npc].pos;
        let combat =
            super::super::player_combat(&stats, &PlayerLoad::fresh("human_mystic")).unwrap();
        zone.spawn(
            zone.next_tick,
            Spawned {
                id: player,
                kind: EntityKind::Player,
                name: "Looter",
                pos: npc_pos,
                speed: Speed::from_milli_tiles_per_tick(0),
                generation: SessionGeneration(1),
                combat: Some(combat),
            },
        )
        .unwrap();
        zone.hate
            .entry(npc)
            .or_default()
            .add(stats.constants(), player, 100, 100);
        let tick = zone.next_tick;
        // B16 (story 3.4): the expected award from the same snapshot, before the kill.
        let expected = zone
            .reward_awards(npc, &rules, rules.reward_npc(pinned).unwrap())
            .unwrap();
        assert_eq!(expected.len(), 1, "a solo attacker");
        assert_eq!(expected[0].player, player);
        let words = zone.rng.get_word_pos();
        let mut events = Vec::new();
        zone.kill(
            tick,
            npc,
            Some(super::super::combat_phase::Blow { attacker: player }),
            &mut events,
        );
        zone.take_phase5_fault().unwrap();
        assert!(zone.rng.get_word_pos() > words, "the death list always draws");
        let block = &zone.phase5.as_ref().unwrap().canonical;
        let spawns: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::EntitySpawn {
                    entity,
                    kind: EntityKind::GroundItem,
                    phase5: Some(detail),
                    ..
                } => Some((*entity, detail.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(spawns.len(), block.ground.len());
        let died = events
            .iter()
            .position(|e| matches!(e, ZoneEvent::EntityDied { .. }))
            .unwrap();
        let first_spawn = events.iter().position(|e| {
            matches!(
                e,
                ZoneEvent::EntitySpawn {
                    kind: EntityKind::GroundItem,
                    ..
                }
            )
        });
        if let Some(first) = first_spawn {
            assert!(first > died, "loot spawns follow EntityDied");
        }
        for (entity, detail) in &spawns {
            let ground = &block.ground[entity];
            assert_eq!(ground.entity, GroundItem::entity_id(ZoneId(75), 5, ground.item.id, tick));
            assert_eq!(detail.item_instance_id, ground.item.id);
            assert_eq!(ground.item.location, ItemLocation::Ground);
            assert_eq!(ground.loot_owner.unwrap().owner, player);
            assert_eq!(ground.loot_owner.unwrap().until, Tick(tick.0 + 150));
            assert_eq!(ground.drop_protection.unwrap().until, Tick(tick.0 + 150));
            assert!(ground.decay_due.is_some());
            let dx = (ground.pos.x.raw() - npc_pos.x.raw()).abs();
            let dy = (ground.pos.y.raw() - npc_pos.y.raw()).abs();
            assert!(dx <= 2187 && dy <= 2187, "offsets within m(70)");
            assert_eq!(zone.entities[entity].kind, EntityKind::GroundItem);
        }
        // Step 7: exactly the B16 award, not the template's killing-blow XP.
        let gained: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::XpGained { entity, amount, .. } if *entity == player => Some(*amount),
                _ => None,
            })
            .collect();
        if expected[0].exp > 0 {
            assert_eq!(gained, vec![expected[0].exp]);
        } else {
            assert!(gained.is_empty());
        }
        assert!(zone.phase5.as_ref().unwrap().rewards.is_none(), "the stash is consumed");
        // V7 corpse time: 10 × corpseTime, not extended (not spoiled, not seeded).
        let corpse_time = rules.drop_npc(pinned).unwrap().corpse_time;
        assert_eq!(
            zone.entities[&npc].ai.as_ref().unwrap().corpse_until,
            Some(Tick(tick.0 + 10 * corpse_time))
        );
        // The block stays valid against the zone (P1 ids, entities, definitions, uniqueness).
        zone.validate_phase5_block().unwrap();
        // Protections clear at their deadline; decay removes everything on its grid point.
        let deadline = Tick(tick.0 + 150);
        let mut out = Vec::new();
        zone.expire_phase5(deadline, &mut out);
        assert!(zone
            .phase5
            .as_ref()
            .unwrap()
            .canonical
            .ground
            .values()
            .all(|g| g.loot_owner.is_none() && g.drop_protection.is_none()));
        if let Some(due) = zone
            .phase5
            .as_ref()
            .unwrap()
            .canonical
            .ground
            .values()
            .filter_map(|g| g.decay_due)
            .max()
        {
            zone.expire_phase5(due, &mut out);
            assert!(zone.phase5.as_ref().unwrap().canonical.ground.is_empty());
            assert!(zone
                .entities
                .values()
                .all(|e| e.kind != EntityKind::GroundItem));
            assert_eq!(
                out.iter()
                    .filter(|e| matches!(e, ZoneEvent::EntityDespawn { .. }))
                    .count(),
                spawns.len()
            );
        }
    }

    #[test]
    fn decay_deadlines_match_the_oracle_d_t_vectors() {
        let rules = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-drops.json"
        ))
        .unwrap();
        for (family, template) in [
            ("loot", 1864_u32),
            ("herb", 8600),
            ("auto_destroy_9849", 9849),
        ] {
            for (dropped, due) in oracle["dt_vectors"][family].as_object().unwrap() {
                let dropped: u64 = dropped.parse().unwrap();
                assert_eq!(
                    decay_due(&rules, template, Tick(dropped)),
                    Some(Tick(due.as_u64().unwrap())),
                    "{family} {dropped}"
                );
            }
        }
        assert!(oracle["dt_vectors"]["pk_drop"].is_null(), "PK drops never decay");
    }

    #[test]
    fn decay_deadlines_follow_the_strict_five_second_grid() {
        let rules = crate::infrastructure::economy_data::embedded_phase5_rules().unwrap();
        // D-T: drops at 0, 1, 49, 50, 51 with the 600 s general limit (6,000 ticks): strict
        // `now − drop > limit` on the 50-tick grid.
        for (dropped, due) in [(0, 6050), (1, 6050), (49, 6050), (50, 6100), (51, 6100)] {
            assert_eq!(decay_due(&rules, 1864, Tick(dropped)), Some(Tick(due)), "{dropped}");
        }
        // Herbs 60 s, item 9849's own 10 s.
        assert_eq!(decay_due(&rules, 8600, Tick(0)), Some(Tick(650)));
        assert_eq!(decay_due(&rules, 9849, Tick(0)), Some(Tick(150)));
        assert_eq!(milli(70), 2187);
        assert_eq!(milli(-70), -2187);
        assert_eq!(milli(-1), -31);
    }
}
