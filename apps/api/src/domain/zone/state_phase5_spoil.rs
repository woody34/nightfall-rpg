//! V7 Spoil and sweep (story 3.6, design §5; B11): the `Spoil` instant effect of skill 254,
//! the `NPC_BODY` target type, the `canSweep` start condition and the `Sweeper` +
//! `ConsumeBody` effects of skill 42, admitted by the P5-D19 V7 skill view. Pinned
//! `Spoil.java`, `ConditionPlayerCanSweep.java:50-75`, `Sweeper.java:44-70`,
//! `ConsumeBody.java:42-47`, `L2Attackable.java:1012-1035` and `L2Party.distributeItem`.

use crate::domain::zone::effects::TargetBinding;
use crate::domain::zone::item_inventory::{count_cap, inventory_totals};
use crate::domain::zone::phase4_projection::ItemOperation;
use crate::domain::zone::phase5_command::LootMode;
use crate::domain::zone::phase5_event::{EconomyBody, EconomyEvent};
use crate::domain::zone::skill::SkillDef;
use crate::domain::zone::{
    CastId, EntityId, EntityKind, ItemCreationOrigin, ItemInstance, ItemLocation, RejectReason,
    SkillOutcome, SkillResolutionKind, SkillTargetResult, Tick, ZoneEvent, ZoneState,
};

use super::phase5_pickup::CharacterRow;

/// `isOldCorpse(player, 2000 ms)` (`L2Attackable.java:1012-1020`; `GC/npc.properties:94`):
/// a sweep needs at least 20 ticks of corpse left.
const SWEEP_MIN_CORPSE_TICKS: u64 = 20;

/// The effect names of the B11 handlers.
pub(super) fn v7_instant_handler(name: &str) -> bool {
    matches!(name, "Spoil" | "Sweeper" | "ConsumeBody" | "Sow" | "Harvesting")
}

fn square(x: i64) -> i64 {
    x.saturating_mul(x)
}

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

#[allow(clippy::needless_pass_by_value)] // map_err adapter.
fn invariant<E>(_: E) -> super::TickError {
    super::TickError::Phase5Invariant
}

impl ZoneState {
    /// `checkSpoilOwner(sweeper)` (`L2Attackable.java:1027-1035`): the spoiler itself, or the
    /// spoiler online and in the sweeper's party (command channels are vacuous).
    fn spoil_owner_v7(&self, sweeper: EntityId, spoiler: EntityId) -> bool {
        if sweeper == spoiler {
            return true;
        }
        let online = self
            .entities
            .get(&spoiler)
            .is_some_and(|e| e.kind == EntityKind::Player);
        let party = |id: EntityId| {
            self.phase5
                .as_ref()
                .and_then(|p| p.canonical.players.get(&id))
                .and_then(|p| p.party)
        };
        online && party(sweeper).is_some() && party(sweeper) == party(spoiler)
    }

    /// `checkInventorySlotsAndWeight(holders)`: the weight of every holder fits the load
    /// ceiling, and the new stacks fit the ordinary/quest pools.
    fn sweep_capacity_v7(&self, sweeper: EntityId, holders: &[(u32, u64)]) -> bool {
        let Some(items) = self.phase4.as_ref() else {
            return false;
        };
        let Some(player) = items.canonical.players.get(&sweeper) else {
            return false;
        };
        let Ok((ordinary, quest, weight)) = inventory_totals(&player.inventory, &items.rules)
        else {
            return false;
        };
        let (mut ordinary, mut quest, mut weight) = (ordinary, quest, i128::from(weight));
        for (template, count) in holders {
            let Some(definition) = items.rules.parts().items.get(template) else {
                return false;
            };
            weight = weight
                .saturating_add(i128::from(definition.weight).saturating_mul(i128::from(*count)));
            let stacked = definition.stackable
                && player.inventory.values().any(|i| {
                    i.template_id == *template
                        && i.enchant == 0
                        && i.augment_id.is_none()
                        && i.charges == 0
                });
            let slots = if stacked {
                0
            } else if definition.stackable {
                1
            } else {
                *count
            };
            if definition
                .flags
                .get("is_questitem")
                .copied()
                .unwrap_or(false)
            {
                quest = quest.saturating_add(slots);
            } else {
                ordinary = ordinary.saturating_add(slots);
            }
        }
        weight <= i128::from(player.max_load)
            && ordinary <= u64::from(player.ordinary_capacity)
            && quest <= u64::from(player.quest_capacity)
    }

    /// Every fresh instance a sweep can create is allocatable before the holders are taken
    /// (Phase 4 I1; z-review-4 P2-1 applied to sweeps): an upper bound of one per stackable
    /// holder (nine for an even split, the party cap) and `count` per non-stackable holder.
    fn sweep_ids_available_v7(&self, holders: &[(u32, u64)]) -> bool {
        let Some(items) = self.phase4.as_ref() else {
            return false;
        };
        let Some(phase5) = self.phase5.as_ref() else {
            return false;
        };
        let even = &phase5.rules.parts().party.party.even_distribute;
        let needed = holders.iter().try_fold(0_u64, |total, (template, count)| {
            let stackable = items.rules.parts().items.get(template)?.stackable;
            let n = if !stackable {
                *count
            } else if *count > 1 && even.items.contains(template) {
                u64::try_from(crate::domain::zone::phase5::PARTY_SIZE_CAP).unwrap_or(u64::MAX)
            } else {
                1
            };
            Some(total.saturating_add(n))
        });
        let (_, next, end) = items.canonical.allocator.bounds();
        needed.is_some_and(|needed| needed <= end.saturating_sub(next))
    }

    /// The `canSweep` start condition (`ConditionPlayerCanSweep.java:50-75`): a spoiled corpse
    /// with holders, the spoil owner, not an old corpse, and slots and weight for the holders.
    pub(super) fn can_sweep_v7(
        &self,
        tick: Tick,
        sweeper: EntityId,
        npc: EntityId,
    ) -> Result<(), RejectReason> {
        let sidecar = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.npcs.get(&npc))
            .ok_or(RejectReason::SweepNotAllowed)?;
        // z-review-5 P2-1: pinned `canSweep` needs `isSpoiled()` only; a spoiled corpse
        // whose roll produced nothing is an empty holder list (the cast still runs).
        let Some(spoiler) = sidecar.spoiler else {
            return Err(RejectReason::SweepNotAllowed);
        };
        let holders: &[(u32, u64)] = sidecar.sweep.as_deref().unwrap_or_default();
        let corpse = self
            .entities
            .get(&npc)
            .and_then(|e| e.ai.as_ref())
            .and_then(|ai| ai.corpse_until)
            .ok_or(RejectReason::SweepNotAllowed)?;
        if !self.spoil_owner_v7(sweeper, spoiler)
            || corpse.0.saturating_sub(tick.0) < SWEEP_MIN_CORPSE_TICKS
        {
            return Err(RejectReason::SweepNotAllowed);
        }
        // z-review-5 P3-3: allocator room is checked here too, so a sweep never starts only
        // to lose its holders at the impact for a Nightfall-only reason.
        if !self.sweep_capacity_v7(sweeper, holders) || !self.sweep_ids_available_v7(holders) {
            return Err(RejectReason::InventoryFull);
        }
        Ok(())
    }

    /// A V7 skill whose only blocker-carrying condition is `canSweep`: the condition is checked
    /// here and stripped before the shared cast-term evaluation (which knows no corpse state).
    pub(super) fn strip_can_sweep_v7(
        &self,
        tick: Tick,
        caster: EntityId,
        target: Option<EntityId>,
        skill: &SkillDef,
    ) -> Result<Option<SkillDef>, RejectReason> {
        use crate::domain::zone::skill::SkillCondition;
        let is_can_sweep = |condition: &SkillCondition| {
            matches!(condition, SkillCondition::Unsupported { tag, attrs }
                if tag == "player" && attrs.get("canSweep").map(String::as_str) == Some("true"))
        };
        if self.phase5.is_none() || !skill.conditions.iter().any(is_can_sweep) {
            return Ok(None);
        }
        self.can_sweep_v7(tick, caster, target.ok_or(RejectReason::InvalidTargetType)?)?;
        let mut stripped = skill.clone();
        stripped.conditions.retain(|c| !is_can_sweep(c));
        Ok(Some(stripped))
    }

    /// One B11 instant handler at the impact, in XML effect order (design §2.1 phase 5).
    #[allow(clippy::too_many_arguments)] // The impact's explicit inputs.
    pub(super) fn v7_instant_v7(
        &mut self,
        tick: Tick,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
        cast_id: CastId,
        name: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<SkillTargetResult, super::TickError> {
        let outcome = match name {
            "Spoil" => self.spoil_v7(caster, target, skill, rank)?,
            "Sweeper" => self.sweeper_v7(tick, caster, target, cast_id, events)?,
            "ConsumeBody" => self.consume_body_v7(tick, target, events),
            "Sow" => self.sow_v7(tick, caster, target, cast_id, events)?,
            "Harvesting" => self.harvesting_v7(tick, caster, target, cast_id, events)?,
            _ => return Err(super::TickError::Phase5Invariant),
        };
        let hp = self
            .entities
            .get(&target.entity)
            .and_then(|e| e.combat.as_ref())
            .map_or(0, |c| c.hp);
        Ok(SkillTargetResult {
            target,
            resolution_kind: SkillResolutionKind::Effect,
            outcome,
            rolled: 0,
            cp_absorbed: 0,
            hp_removed: 0,
            heal: 0,
            target_hp_after: hp,
            applied_effect_ids: Vec::new(),
        })
    }

    /// `Spoil` (design §5.1): `calcMagicSuccess` through the V7 rate and the Phase 3
    /// magic-success draw (#1), taken first; then a living monster (else `INCORRECT_TARGET`,
    /// reported as `Miss`), not already spoiled (`Miss`), else `spoiler := caster` and an
    /// attacked notification (`Hit`). A failed success draw is `Resist`.
    fn spoil_v7(
        &mut self,
        caster: TargetBinding,
        target: TargetBinding,
        skill: &SkillDef,
        rank: u32,
    ) -> Result<SkillOutcome, super::TickError> {
        use crate::domain::zone::skill_math;
        let a = self.effective(caster.entity)?;
        let b = self.effective(target.entity)?;
        let magic_level = *skill
            .magic_level
            .get(usize::try_from(rank.saturating_sub(1)).map_err(invariant)?)
            .ok_or(super::TickError::Phase5Invariant)?;
        let difference = i32::try_from(b.physical.level())
            .map_err(invariant)?
            .saturating_sub(if magic_level > 0 {
                i32::try_from(magic_level).map_err(invariant)?
            } else {
                i32::try_from(a.physical.level()).map_err(invariant)?
            });
        let penalty = self.npc_penalty_v5(
            caster.entity,
            target.entity,
            crate::domain::zone::skill_npc::NpcPenaltyPath::MagicFailure,
        )?;
        let resist = self.damage_stat_v5(target.entity, &b, "magicSuccRes", super::super::Q)?;
        let rate =
            skill_math::magic_success_rate_v7(difference, penalty, resist).map_err(invariant)?;
        let roll = super::runtime::draw_damage_v5(
            self,
            crate::domain::zone::state_damage::DamageRollSite::MagicSuccess,
            target,
            100,
        )
        .map_err(invariant)?;
        if i64::from(roll) >= i64::from(rate) {
            return Ok(SkillOutcome::Resist);
        }
        let monster = self.entities.get(&target.entity).is_some_and(|e| {
            e.kind == EntityKind::Npc
                && e.ai.is_some()
                && e.targeting.attackable
                && !e.targeting.dead
        });
        if !monster {
            return Ok(SkillOutcome::Miss);
        }
        let Some(sidecar) = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.npcs.get_mut(&target.entity))
        else {
            return Ok(SkillOutcome::Miss);
        };
        if sidecar.spoiler.is_some() {
            return Ok(SkillOutcome::Miss);
        }
        sidecar.spoiler = Some(caster.entity);
        // `notifyEvent(EVT_ATTACKED, effector)` through the existing AI request path.
        if let Some(phase3) = self.phase3.as_mut() {
            phase3
                .ai_requests
                .push(crate::domain::zone::phase3::AiRequest {
                    npc: target,
                    target: Some(caster),
                    force: false,
                });
        }
        Ok(SkillOutcome::Hit)
    }

    /// `Sweeper` (`Sweeper.java:44-70`): re-check the spoil owner and slots/weight; on
    /// success take the holders and route each one (solo: `addItem("Sweeper")`; party:
    /// `distributeItem(.., spoil = true, npc)`), each a fresh instance or a merge (§5.2).
    fn sweeper_v7(
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
        let Some(spoiler) = sidecar.spoiler else {
            return Ok(SkillOutcome::Miss);
        };
        let holders = sidecar.sweep.clone().unwrap_or_default();
        if !self.spoil_owner_v7(caster.entity, spoiler)
            || !self.sweep_capacity_v7(caster.entity, &holders)
            || !self.sweep_ids_available_v7(&holders)
        {
            return Ok(SkillOutcome::Miss);
        }
        if let Some(npc) = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.npcs.get_mut(&target.entity))
        {
            npc.sweep = None;
        }
        let origin = self
            .entities
            .get(&target.entity)
            .map(|e| {
                (
                    e.pos,
                    e.combat
                        .as_ref()
                        .map_or(0, |c| i64::from(c.collision_radius.raw())),
                )
            })
            .ok_or(super::TickError::Phase5Invariant)?;
        let resolution = format!("sweep:{}", cast_id.0);
        for (template, count) in holders {
            self.route_sweep_holder_v7(
                tick,
                caster.entity,
                template,
                count,
                origin,
                &resolution,
                events,
            )
            .map_err(|_| super::TickError::Phase5Invariant)?;
        }
        self.corpse_state_v7(tick, target.entity, true, events);
        Ok(SkillOutcome::Hit)
    }

    /// One swept holder through `L2Party.distributeItem(sweeper, holder, true, npc)` or solo
    /// `addItem("Sweeper")` (design §4.2 sweep routing).
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Pinned routing, one place.
    fn route_sweep_holder_v7(
        &mut self,
        tick: Tick,
        sweeper: EntityId,
        template: u32,
        count: u64,
        (origin, origin_radius): (crate::domain::zone::Vec2Fixed, i64),
        resolution: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let phase5 = self.phase5.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let rules = std::sync::Arc::clone(&phase5.rules);
        let party = phase5
            .canonical
            .players
            .get(&sweeper)
            .and_then(|p| p.party)
            .and_then(|id| phase5.canonical.parties.get(&id))
            .cloned();
        let Some(party) = party else {
            return self.sweep_payout_v7(
                tick,
                sweeper,
                template,
                count,
                ItemOperation::Sweep,
                resolution,
                &[],
                events,
            );
        };
        let range2 = i64::try_from(rules.parts().party.party.party_range2)
            .map_err(invalid)?
            .saturating_mul(1000)
            / 32;
        // `checkIfInRange(PartyRange2, npc, member, true)`: rad = npc + member (0) radius.
        let bound = square(range2.saturating_mul(2).saturating_add(origin_radius));
        let in_range = |zone: &Self, member: EntityId| {
            zone.entities.get(&member).is_some_and(|e| {
                let dx = i64::from(origin.x.raw()).saturating_sub(i64::from(e.pos.x.raw()));
                let dy = i64::from(origin.y.raw()).saturating_sub(i64::from(e.pos.y.raw()));
                square(dx).saturating_add(square(dy)).saturating_mul(4) <= bound
            })
        };
        let stackable = self
            .phase4
            .as_ref()
            .and_then(|items| items.rules.parts().items.get(&template))
            .ok_or(RejectReason::InvalidLoad)?
            .stackable;
        let even = &rules.parts().party.party.even_distribute;
        if count > 1
            && (even.items.contains(&template)
                || (stackable && even.all_stackable_items)
                || (!stackable && even.all_other_items))
        {
            // `distributeItemEvenly` from the NPC: in-range members in member order.
            let recipients: Vec<EntityId> = party
                .members
                .iter()
                .copied()
                .filter(|m| in_range(self, *m))
                .collect();
            if recipients.is_empty() {
                // Pinned: nobody in range receives anything (the holder never existed).
                return Ok(());
            }
            let k = u64::try_from(recipients.len()).map_err(invalid)?;
            let mut shares =
                vec![count.checked_div(k).ok_or(RejectReason::InvalidLoad)?; recipients.len()];
            for _ in 0..count.checked_rem(k).ok_or(RejectReason::InvalidLoad)? {
                let before = self.rng.get_word_pos();
                let chosen = self.roll_below(u32::try_from(k).map_err(invalid)?);
                self.label_raw_draw(before, "party_split_remainder");
                let slot = shares
                    .get_mut(usize::try_from(chosen).map_err(invalid)?)
                    .ok_or(RejectReason::InvalidLoad)?;
                *slot = slot.saturating_add(1);
            }
            // P5-D33: the recipient set (shares > 0), canonical ids ascending.
            let mut set: Vec<uuid::Uuid> = recipients
                .iter()
                .zip(&shares)
                .filter(|(_, share)| **share > 0)
                .map(|(r, _)| r.as_uuid())
                .collect();
            set.sort_unstable();
            for (recipient, share) in recipients.into_iter().zip(shares) {
                if share > 0 {
                    self.sweep_payout_v7(
                        tick,
                        recipient,
                        template,
                        share,
                        ItemOperation::PartySplit,
                        resolution,
                        &set,
                        events,
                    )?;
                }
            }
            return Ok(());
        }
        // `getActualLooter(.., spoil = true, npc)`: FINDERS_KEEPERS, RANDOM and BY_TURN keep
        // the sweeper; the *_INCLUDING_SPOIL modes draw or advance like a pickup.
        let looter = match party.distribution {
            LootMode::FindersKeepers | LootMode::Random | LootMode::ByTurn => sweeper,
            LootMode::RandomIncludingSpoil => {
                let candidates: Vec<EntityId> = party
                    .members
                    .iter()
                    .copied()
                    .filter(|m| in_range(self, *m))
                    .collect();
                if candidates.is_empty() {
                    sweeper
                } else {
                    let before = self.rng.get_word_pos();
                    let index = self.roll_below(u32::try_from(candidates.len()).map_err(invalid)?);
                    self.label_raw_draw(before, "party_random");
                    candidates
                        .get(usize::try_from(index).map_err(invalid)?)
                        .copied()
                        .ok_or(RejectReason::InvalidLoad)?
                }
            },
            LootMode::ByTurnIncludingSpoil => {
                let len = u32::try_from(party.members.len()).map_err(invalid)?;
                let mut cursor = party.item_last_loot;
                let mut winner = None;
                for _ in 0..len {
                    cursor = cursor.saturating_add(1);
                    if cursor >= len {
                        cursor = 0;
                    }
                    let member = party
                        .members
                        .get(usize::try_from(cursor).map_err(invalid)?)
                        .copied()
                        .ok_or(RejectReason::InvalidLoad)?;
                    if in_range(self, member) {
                        winner = Some(member);
                        break;
                    }
                }
                if let Some(stored) = self
                    .phase5
                    .as_mut()
                    .and_then(|p| p.canonical.parties.get_mut(&party.id))
                {
                    stored.item_last_loot = cursor;
                }
                winner.unwrap_or(sweeper)
            },
        };
        self.sweep_payout_v7(
            tick,
            looter,
            template,
            count,
            ItemOperation::Sweep,
            resolution,
            &[looter.as_uuid()],
            events,
        )?;
        let audience: Vec<EntityId> = party
            .members
            .iter()
            .copied()
            .filter(|m| *m != looter)
            .collect();
        events.push(ZoneEvent::Economy(Box::new(EconomyEvent {
            tick,
            entity: sweeper,
            body: EconomyBody::LootDistributed {
                recipient: looter,
                template_id: template,
                count,
                mode: party.distribution,
                spoil: true,
            },
            audience,
        })));
        Ok(())
    }

    /// One swept payout: a fresh instance, or a merge into the recipient's stack, with its
    /// `sweep` / `party_split` row (cause `sweep:<cast_id>`) in the recipient's projection.
    /// A count cap violation drops the payout (pinned `addItem` clamps; D68 refuses growth).
    #[allow(clippy::too_many_arguments)] // The payout's explicit inputs.
    fn sweep_payout_v7(
        &mut self,
        tick: Tick,
        recipient: EntityId,
        template: u32,
        count: u64,
        operation: ItemOperation,
        resolution: &str,
        recipients: &[uuid::Uuid],
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let origin = ItemCreationOrigin {
            zone: self.seed.zone,
            epoch: self.seed.epoch,
        };
        let initial_changed = events
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(p) if p.entity == recipient));
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
            .get(&recipient)
            .ok_or(RejectReason::InvalidLoad)?;
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
        // A non-stackable holder becomes `count` instances of 1 each (always within cap).
        if stackable
            && existing
                .as_ref()
                .map_or(0, |i| i.count)
                .checked_add(count)
                .is_none_or(|total| total > count_cap(template, stackable))
        {
            return Ok(());
        }
        // D79: one revision step per changed owner-tick.
        let revision = if initial_changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let mut rows = Vec::new();
        if let Some(before) = existing {
            let mut after = before.clone();
            after.count = after.count.saturating_add(count);
            after.revision = revision;
            rows.push((Some(before), after, i64::try_from(count).map_err(invalid)?));
        } else {
            let pieces = if stackable { 1 } else { count };
            for _ in 0..pieces {
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
            .get_mut(&recipient)
            .ok_or(RejectReason::InvalidLoad)?;
        for (_, after, _) in &rows {
            after.validate().map_err(invalid)?;
            player.inventory.insert(after.id, after.clone());
        }
        player.inventory_revision = revision;
        let (_, _, weight) = inventory_totals(&player.inventory, &rules).map_err(invalid)?;
        player.weight = weight;
        self.rebuild_v5(recipient).map_err(invalid)?;
        for (before, after, count_delta) in rows {
            self.push_character_row(
                tick,
                recipient,
                CharacterRow {
                    operation,
                    before,
                    after: Some(after),
                    count_delta,
                    receipt: None,
                    cause: Some(resolution.to_owned()),
                    transfer: None,
                    recipients: recipients.to_vec(),
                },
                events,
            )?;
        }
        Ok(())
    }

    /// `ConsumeBody` (`ConsumeBody.java:42-47`, `endDecayTask`): the corpse ends now and any
    /// untaken holders vanish (no instances existed); the entity leaves in the next spawn
    /// phase. The respawn tick is not moved (rule P2).
    fn consume_body_v7(
        &mut self,
        tick: Tick,
        target: TargetBinding,
        events: &mut Vec<ZoneEvent>,
    ) -> SkillOutcome {
        let Some(ai) = self
            .entities
            .get_mut(&target.entity)
            .and_then(|e| e.ai.as_mut())
        else {
            return SkillOutcome::Miss;
        };
        if ai.corpse_until.is_none_or(|until| until <= tick) {
            return SkillOutcome::Miss;
        }
        ai.corpse_until = Some(tick);
        let swept = self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.npcs.get_mut(&target.entity))
            .is_some_and(|npc| npc.sweep.take().is_some());
        if swept {
            self.corpse_state_v7(tick, target.entity, false, events);
        }
        SkillOutcome::Hit
    }

    /// `CorpseStateChanged{spoiled, seeded, sweepable, swept}` (design §5.3), public.
    pub(super) fn corpse_state_v7(
        &self,
        tick: Tick,
        npc: EntityId,
        swept: bool,
        events: &mut Vec<ZoneEvent>,
    ) {
        let Some(sidecar) = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.npcs.get(&npc))
        else {
            return;
        };
        events.push(ZoneEvent::Economy(Box::new(EconomyEvent {
            tick,
            entity: npc,
            body: EconomyBody::CorpseStateChanged {
                spoiled: sidecar.spoiler.is_some(),
                seeded: sidecar.seeded,
                sweepable: sidecar.sweep.is_some(),
                swept,
            },
            audience: Vec::new(),
        })));
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::zone::phase5::{Party, PartyId};
    use crate::domain::zone::skill::SkillId;
    use crate::infrastructure::zone_data;

    use super::super::item_materialize::tests::{fresh_zone_and_kit_load, spawn_kit};

    const P1: u128 = 71;
    const P2: u128 = 81;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// A V7 zone with two admitted kit players and one living pinned monster (npc 102).
    fn zone() -> (ZoneState, EntityId) {
        let (mut zone, mut load, _) = fresh_zone_and_kit_load(true);
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        let mut second = load.clone();
        if let Some(progression) = second.progression.as_mut() {
            progression.identity.account_id =
                crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(82));
        }
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(id(P1), load), spawn_kit(id(P2), second)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let stats = zone.rules.clone().unwrap();
        let def = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
        let mut specs = crate::application::zone_bootstrap::slot_specs(&stats, &def).unwrap();
        specs.truncate(1);
        specs[0].count = 1;
        specs[0].pinned_npc_id = Some(102);
        zone = zone.try_with_spawn_slots(specs).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let npc = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Npc && e.ai.is_some())
            .unwrap()
            .id;
        (zone, npc)
    }

    fn skill(zone: &ZoneState, id: u32) -> SkillDef {
        zone.phase3
            .as_ref()
            .unwrap()
            .canonical
            .rules
            .skills()
            .get(SkillId(id))
            .unwrap()
            .clone()
    }

    fn binding(zone: &ZoneState, entity: EntityId) -> TargetBinding {
        zone.binding(entity).unwrap()
    }

    /// Kill the monster in place: dead, corpse up for `ticks`, spoiled by `spoiler` with
    /// `holders`.
    fn corpse(
        zone: &mut ZoneState,
        npc: EntityId,
        ticks: u64,
        spoiler: EntityId,
        holders: Vec<(u32, u64)>,
    ) {
        let until = Tick(zone.next_tick.0 + ticks);
        let entity = zone.entities.get_mut(&npc).unwrap();
        entity.targeting.dead = true;
        entity.ai.as_mut().unwrap().corpse_until = Some(until);
        let sidecar = zone
            .phase5
            .as_mut()
            .unwrap()
            .canonical
            .npcs
            .get_mut(&npc)
            .unwrap();
        sidecar.spoiler = Some(spoiler);
        sidecar.sweep = Some(holders);
    }

    fn held(zone: &ZoneState, who: u128, template: u32) -> u64 {
        zone.phase4.as_ref().unwrap().canonical.players[&id(who)]
            .inventory
            .values()
            .filter(|i| i.template_id == template)
            .map(|i| i.count)
            .sum()
    }

    /// Spoil (design §5.1): the magic-success draw comes first and is always taken; a
    /// success marks the living monster spoiled by the caster; a second Spoil finds it
    /// already spoiled (`Miss`) after its own draw.
    #[test]
    fn spoil_draws_first_then_marks_the_monster_once() {
        let (mut zone, npc) = zone();
        let spoil = skill(&zone, 254);
        let (caster, target) = (binding(&zone, id(P1)), binding(&zone, npc));
        let tick = zone.next_tick;
        let mut events = Vec::new();
        let mut outcomes = Vec::new();
        for _ in 0..40 {
            let before = zone.rng.get_word_pos();
            let result = zone
                .v7_instant_v7(tick, caster, target, &spoil, 1, CastId(1), "Spoil", &mut events)
                .unwrap();
            assert!(zone.rng.get_word_pos() > before, "the success draw is always taken");
            outcomes.push(result.outcome);
            if result.outcome == SkillOutcome::Hit {
                break;
            }
            assert_eq!(result.outcome, SkillOutcome::Resist);
        }
        assert_eq!(outcomes.last(), Some(&SkillOutcome::Hit), "{outcomes:?}");
        let sidecar = &zone.phase5.as_ref().unwrap().canonical.npcs[&npc];
        assert_eq!(sidecar.spoiler, Some(id(P1)));
        // Already spoiled: `Miss` on success, `Resist` otherwise; the spoiler never changes.
        let other = binding(&zone, id(P2));
        for _ in 0..10 {
            let result = zone
                .v7_instant_v7(tick, other, target, &spoil, 1, CastId(2), "Spoil", &mut events)
                .unwrap();
            assert!(matches!(result.outcome, SkillOutcome::Miss | SkillOutcome::Resist));
        }
        assert_eq!(zone.phase5.as_ref().unwrap().canonical.npcs[&npc].spoiler, Some(id(P1)));
    }

    /// `canSweep` (design §5.1): spoiled with holders, the spoil owner or the owner's party,
    /// at least 20 corpse ticks left, and room for the holders.
    #[test]
    fn can_sweep_needs_the_owner_a_fresh_corpse_and_room() {
        let (mut zone, npc) = zone();
        let tick = zone.next_tick;
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 10)]);
        zone.can_sweep_v7(tick, id(P1), npc).unwrap();
        assert_eq!(zone.can_sweep_v7(tick, id(P2), npc), Err(RejectReason::SweepNotAllowed));
        // The spoiler's party may sweep.
        let block = &mut zone.phase5.as_mut().unwrap().canonical;
        block.parties.insert(
            PartyId(1),
            Party {
                id: PartyId(1),
                members: vec![id(P1), id(P2)],
                distribution: LootMode::FindersKeepers,
                item_last_loot: 0,
                loot_vote: None,
                pending_invite: None,
            },
        );
        block.next_party = 2;
        for member in [P1, P2] {
            block.players.get_mut(&id(member)).unwrap().party = Some(PartyId(1));
        }
        zone.can_sweep_v7(tick, id(P2), npc).unwrap();
        // An old corpse (19 ticks left) or an unspoiled one refuses.
        corpse(&mut zone, npc, 19, id(P1), vec![(57, 10)]);
        assert_eq!(zone.can_sweep_v7(tick, id(P1), npc), Err(RejectReason::SweepNotAllowed));
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 10)]);
        zone.phase5
            .as_mut()
            .unwrap()
            .canonical
            .npcs
            .get_mut(&npc)
            .unwrap()
            .spoiler = None;
        assert_eq!(zone.can_sweep_v7(tick, id(P1), npc), Err(RejectReason::SweepNotAllowed));
        // z-review-5 P2-1: spoiled with an empty roll is still sweepable.
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 10)]);
        zone.phase5
            .as_mut()
            .unwrap()
            .canonical
            .npcs
            .get_mut(&npc)
            .unwrap()
            .sweep = None;
        zone.can_sweep_v7(tick, id(P1), npc).unwrap();
        // Too heavy for the load ceiling.
        corpse(&mut zone, npc, 100, id(P1), vec![(1, 100_000)]);
        assert_eq!(zone.can_sweep_v7(tick, id(P1), npc), Err(RejectReason::InventoryFull));
        // The canSweep leaf is stripped only for a skill that carries it, after the check.
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 10)]);
        let sweeper = skill(&zone, 42);
        let stripped = zone
            .strip_can_sweep_v7(tick, id(P1), Some(npc), &sweeper)
            .unwrap()
            .unwrap();
        assert!(stripped.conditions.is_empty());
        assert!(zone
            .strip_can_sweep_v7(tick, id(P1), Some(npc), &skill(&zone, 254))
            .unwrap()
            .is_none());
    }

    /// The cast pipeline admits Sweeper (V7 skill view, `NPC_BODY`, `canSweep`) on a fresh
    /// spoiled corpse within 20 L2 units, and refuses a living target, a non-owner and an
    /// out-of-range body before anything starts.
    #[test]
    fn sweeper_casts_start_only_on_an_owned_fresh_corpse_in_range() {
        use crate::domain::zone::state_skill::{CapturedTarget, StagedCast};
        let (mut zone, npc) = zone();
        let here = zone.entities[&id(P1)].pos;
        let from = zone.entities[&npc].pos;
        zone.entities.get_mut(&npc).unwrap().pos = here;
        zone.aoi.relocate(npc, from, here);
        let tick = zone.next_tick;
        let sweeper = skill(&zone, 42);
        let start = |zone: &mut ZoneState, who: u128| {
            let caster = binding(zone, id(who));
            let target = binding(zone, npc);
            let mut events = Vec::new();
            zone.start_defined_cast_v6(
                tick,
                &StagedCast {
                    ordinal: zone.next_ordinal,
                    caster,
                    skill_id: SkillId(42),
                    target: Some(CapturedTarget {
                        entity: npc,
                        incarnation: Some(target.incarnation),
                    }),
                    canceled: None,
                },
                &sweeper,
                crate::domain::zone::skill::SkillRef {
                    id: SkillId(42),
                    rank: 1,
                },
                None,
                None,
                &mut events,
            )
        };
        // A living monster is not a body.
        assert_eq!(start(&mut zone, P1), Err(RejectReason::InvalidTargetType));
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 10)]);
        assert_eq!(start(&mut zone, P2), Err(RejectReason::SweepNotAllowed));
        start(&mut zone, P1).unwrap();
        assert!(matches!(
            zone.phase3.as_ref().unwrap().canonical.entities[&id(P1)].cast,
            crate::domain::zone::CastState::Casting(_)
        ));
    }

    /// Coordinator / z-review-6 P1-1: Spoil and Sweeper through `ZoneCommand::CastSkill` →
    /// launch → impact for a level-10 dwarven fighter who learned both (pinned tree, level
    /// 10), with the T55 restore after every tick: Spoil marks the living monster, a real
    /// kill rolls the corpse list, and Sweeper + `ConsumeBody` end the corpse.
    #[test]
    fn spoil_and_sweeper_run_through_cast_skill_launch_and_impact() {
        use crate::domain::subclass::LearnedSkill;
        use crate::domain::zone::{SessionGeneration, ZoneCommand, ZoneInput};
        let (mut zone, mut load, me) =
            super::super::item_materialize::tests::kit_load_for(true, Some(53));
        let stats = zone.rules.clone().unwrap();
        load.level = 10;
        load.xp = stats.xp_table()[9];
        if let Some(progression) = load.progression.as_mut() {
            progression.class_state.learned_skills = vec![
                LearnedSkill {
                    key: "l2.skill.254".into(),
                    level: 1,
                },
                LearnedSkill {
                    key: "l2.skill.42".into(),
                    level: 1,
                },
            ];
        }
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(me, load)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        let def = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
        let mut specs = crate::application::zone_bootstrap::slot_specs(&stats, &def).unwrap();
        specs.truncate(1);
        specs[0].count = 1;
        specs[0].pinned_npc_id = Some(20_481);
        zone = zone.try_with_spawn_slots(specs).unwrap();
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let npc = zone
            .entities
            .values()
            .find(|e| e.kind == EntityKind::Npc && e.ai.is_some())
            .unwrap()
            .id;
        let here = zone.entities[&me].pos;
        let from = zone.entities[&npc].pos;
        let body = zone.entities.get_mut(&npc).unwrap();
        body.pos = here;
        body.ai.as_mut().unwrap().home = here;
        zone.aoi.relocate(npc, from, here);
        let mut seq = 1;
        let mut send = |zone: &mut ZoneState, command: ZoneCommand| {
            seq += 1;
            let tick = zone
                .run_tick(zone.draft(vec![ZoneInput::session(
                    me,
                    SessionGeneration(1),
                    seq,
                    command,
                )]))
                .unwrap();
            assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
            tick
        };
        let result = |tick: &crate::domain::zone::AppliedTick, skill: u32| {
            tick.events.iter().find_map(|e| match e {
                ZoneEvent::SkillCastResult {
                    caster,
                    skill: s,
                    targets,
                    ..
                } if caster.entity == me && s.id.0 == skill => {
                    Some(targets.iter().map(|t| t.outcome).collect::<Vec<_>>())
                },
                _ => None,
            })
        };
        let spoiled = |zone: &ZoneState| {
            zone.phase5
                .as_ref()
                .unwrap()
                .canonical
                .npcs
                .get(&npc)
                .and_then(|n| n.spoiler)
        };
        // Spoil until it lands (reuse 3 s between tries).
        for _ in 0..8 {
            send(
                &mut zone,
                ZoneCommand::CastSkill {
                    entity: me,
                    skill_id: SkillId(254),
                    target: Some(npc),
                },
            );
            let mut outcome = None;
            for _ in 0..80 {
                let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
                if let Some(o) = result(&tick, 254) {
                    outcome = Some(o);
                    break;
                }
            }
            assert!(outcome.is_some(), "Spoil impact within 80 ticks");
            if spoiled(&zone).is_some() {
                break;
            }
            for _ in 0..31 {
                zone.run_tick(zone.draft(Vec::new())).unwrap();
            }
        }
        assert_eq!(spoiled(&zone), Some(me), "spoiled by the caster");
        // Kill it through combat (the V7 death rolls the corpse list into the holders).
        zone.entities
            .get_mut(&npc)
            .unwrap()
            .combat
            .as_mut()
            .unwrap()
            .hp = 1;
        send(
            &mut zone,
            ZoneCommand::SetTarget {
                entity: me,
                target: Some(npc),
            },
        );
        send(&mut zone, ZoneCommand::Attack { entity: me });
        for _ in 0..200 {
            if zone.entities[&npc].targeting.dead {
                break;
            }
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        assert!(zone.entities[&npc].targeting.dead, "killed");
        let corpse = zone.entities[&npc]
            .ai
            .as_ref()
            .unwrap()
            .corpse_until
            .unwrap();
        assert!(corpse.0 >= zone.next_tick.0 + 20, "a fresh, spoiled (extended) corpse");
        send(
            &mut zone,
            ZoneCommand::CastSkill {
                entity: me,
                skill_id: SkillId(42),
                target: Some(npc),
            },
        );
        let mut swept = None;
        for _ in 0..40 {
            let tick = zone.run_tick(zone.draft(Vec::new())).unwrap();
            if let Some(o) = result(&tick, 42) {
                swept = Some((o, tick.tick));
                break;
            }
        }
        let (outcomes, at) = swept.unwrap();
        assert_eq!(outcomes, vec![SkillOutcome::Hit, SkillOutcome::Hit], "Sweeper, ConsumeBody");
        assert!(zone
            .phase5
            .as_ref()
            .unwrap()
            .canonical
            .npcs
            .get(&npc)
            .is_none_or(|n| n.sweep.is_none()));
        assert!(zone.entities.get(&npc).is_none_or(|e| e
            .ai
            .as_ref()
            .and_then(|ai| ai.corpse_until)
            .is_some_and(|until| until <= at)));
    }

    /// `Sweeper` then `ConsumeBody` (design §5.1–§5.3): the holders become fresh instances (or a
    /// merge) with `sweep` rows in the sweeper's projection, the corpse ends this tick and
    /// the holders are gone; `CorpseStateChanged{swept}` is public.
    #[test]
    fn sweeper_takes_fresh_items_and_consume_body_ends_the_corpse() {
        let (mut zone, npc) = zone();
        let tick = zone.next_tick;
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 10), (1, 2)]);
        let sweeper = skill(&zone, 42);
        let (caster, target) = (binding(&zone, id(P1)), binding(&zone, npc));
        let mut events = Vec::new();
        let swept = zone
            .v7_instant_v7(tick, caster, target, &sweeper, 1, CastId(9), "Sweeper", &mut events)
            .unwrap();
        assert_eq!(swept.outcome, SkillOutcome::Hit);
        assert_eq!((held(&zone, P1, 57), held(&zone, P1, 1)), (10, 2));
        let rows: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == id(P1) => Some(p.ledger.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        // One adena stack and two Short Swords (non-stackable), each a fresh `sweep` row.
        assert_eq!(rows.len(), 3);
        assert!(rows
            .iter()
            .all(|r| r.operation == ItemOperation::Sweep && r.before.is_none()));
        let consumed = zone
            .v7_instant_v7(tick, caster, target, &sweeper, 1, CastId(9), "ConsumeBody", &mut events)
            .unwrap();
        assert_eq!(consumed.outcome, SkillOutcome::Hit);
        assert_eq!(
            zone.entities[&npc].ai.as_ref().unwrap().corpse_until,
            Some(tick),
            "the corpse ends at the impact; the entity leaves next spawn phase"
        );
        assert!(zone.phase5.as_ref().unwrap().canonical.npcs[&npc]
            .sweep
            .is_none());
        assert!(events.iter().any(|e| matches!(
            e,
            ZoneEvent::Economy(ev) if matches!(ev.body, EconomyBody::CorpseStateChanged { swept: true, .. })
        )));
        // z-review-5 P2-1: a spoiled corpse with an empty roll: the sweep grants nothing and
        // ConsumeBody still ends the corpse.
        corpse(&mut zone, npc, 100, id(P1), Vec::new());
        zone.phase5
            .as_mut()
            .unwrap()
            .canonical
            .npcs
            .get_mut(&npc)
            .unwrap()
            .sweep = None;
        let before = zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .clone();
        let mut empty = Vec::new();
        let swept = zone
            .v7_instant_v7(tick, caster, target, &sweeper, 1, CastId(11), "Sweeper", &mut empty)
            .unwrap();
        assert_eq!(swept.outcome, SkillOutcome::Hit);
        assert_eq!(zone.phase4.as_ref().unwrap().canonical.players[&id(P1)].inventory, before);
        assert!(!empty
            .iter()
            .any(|e| matches!(e, ZoneEvent::ItemCheckpoint(_))));
        zone.v7_instant_v7(
            tick,
            caster,
            target,
            &sweeper,
            1,
            CastId(11),
            "ConsumeBody",
            &mut empty,
        )
        .unwrap();
        assert_eq!(zone.entities[&npc].ai.as_ref().unwrap().corpse_until, Some(tick));
        // A failed re-check (not the owner) takes nothing; ConsumeBody still clears holders.
        corpse(&mut zone, npc, 100, id(P1), vec![(57, 5)]);
        let other = binding(&zone, id(P2));
        let mut events = Vec::new();
        let failed = zone
            .v7_instant_v7(tick, other, target, &sweeper, 1, CastId(10), "Sweeper", &mut events)
            .unwrap();
        assert_eq!(failed.outcome, SkillOutcome::Miss);
        assert_eq!(held(&zone, P2, 57), 0);
        zone.v7_instant_v7(
            tick,
            other,
            target,
            &sweeper,
            1,
            CastId(10),
            "ConsumeBody",
            &mut events,
        )
        .unwrap();
        assert!(zone.phase5.as_ref().unwrap().canonical.npcs[&npc]
            .sweep
            .is_none());
    }
}
