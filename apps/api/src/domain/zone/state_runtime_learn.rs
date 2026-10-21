//! Live command-ordered learning (design v3 §5.3). The application resolves durable
//! receipts first and serializes mutations/lifecycle work; this module owns the debit,
//! passive installation and final immutable success fact, never a repository or reply.

use super::*;
use crate::domain::zone::item_inventory::{InventoryBoundary, InventoryLimits, InventoryMutation};
use crate::domain::zone::skill::SkillRef;
use crate::domain::zone::state_learn::{prepare_learn, LearnContext, LearnError};
use crate::domain::zone::{FrozenLearnResult, LearnedEffectiveStats};
use crate::domain::{AccountId, CharacterId};

struct PreparedLearning {
    entity: Entity,
    sidecar: Phase3Entity,
    next_sequence: u64,
    spent: u64,
    /// V6 (E3.4/D47) skill-book costs as `(template, count)`; empty for V5 and free rows.
    required_items: Vec<(u32, u32)>,
}

fn learn_reject(error: LearnError) -> RejectReason {
    match error {
        LearnError::InvalidState | LearnError::SequenceExhausted => RejectReason::InvalidLoad,
        LearnError::DeadActor => RejectReason::DeadActor,
        LearnError::InCombat => RejectReason::InCombat,
        LearnError::TrainerRequired => RejectReason::ClassMasterTooFar,
        LearnError::UnknownSkill => RejectReason::SkillUnknown,
        LearnError::NotLearnable => RejectReason::SkillNotLearnable,
        LearnError::WrongRank => RejectReason::SkillWrongRank,
        LearnError::LevelRequired => RejectReason::SkillLevelRequired,
        LearnError::ItemRequired => RejectReason::SkillItemRequired,
        LearnError::DeferredSkill => RejectReason::SkillDeferred,
        LearnError::InsufficientSp => RejectReason::InsufficientSp,
    }
}

impl ZoneState {
    /// Read-only exact-rank eligibility from current authoritative state. Checks account
    /// and all 64 generation bits, then the same guards/stat reconstruction as mutation.
    /// It consumes no SP, IDs, RNG, cache entries or receipts. The actor must recheck when
    /// executing the command; this query is not a reservation or a durable success.
    pub fn validate_learn_skill(
        &self,
        entity: EntityId,
        account: AccountId,
        generation: SessionGeneration,
        skill: SkillRef,
    ) -> Result<(), RejectReason> {
        let prepared = self.prepare_live_learn(entity, account, generation, skill)?;
        self.learn_items_owned(entity, &prepared.required_items)
    }

    /// V6 read-only possession check: the book(s) must be in the owner's inventory now. The
    /// payment takes them through the inventory mutation when the command executes.
    fn learn_items_owned(&self, id: EntityId, required: &[(u32, u32)]) -> Result<(), RejectReason> {
        if required.is_empty() {
            return Ok(());
        }
        let inventory = &self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&id))
            .ok_or(RejectReason::InvalidLoad)?
            .inventory;
        for (template, count) in required {
            let owned = inventory
                .values()
                .filter(|item| item.template_id == *template)
                .try_fold(0_u64, |sum, item| sum.checked_add(item.count))
                .ok_or(RejectReason::InvalidLoad)?;
            if owned < u64::from(*count) {
                return Err(RejectReason::SkillItemRequired);
            }
        }
        Ok(())
    }

    pub(in super::super) fn engaged_for_learning(&self, entity: EntityId) -> bool {
        let Some(e) = self.entities.get(&entity) else {
            return true;
        };
        let Some(c) = e.combat.as_ref() else {
            return true;
        };
        if c.auto_attack
            || c.chasing
            || c.swing.is_some()
            || c.ready_at > self.next_tick
            || self.hate.values().any(|h| h.get(entity).is_some())
            || self.entities.values().any(|other| {
                !other.targeting.dead
                    && other.combat.as_ref().is_some_and(|combat| {
                        (combat.auto_attack && other.targeting.target == Some(entity))
                            || combat.swing.is_some_and(|s| s.target == entity)
                    })
            })
        {
            return true;
        }
        let Some(phase) = &self.phase3 else {
            return true;
        };
        phase.canonical.entities.iter().any(|(id, side)| {
            // Hostile effects engage both the target and a still-present source. Captured
            // life bindings prevent an old NPC incarnation from engaging its replacement.
            side.effects.entries().values().any(|effect| {
                self.hostile_skill_v5(effect.skill.id)
                    && (*id == entity
                        || effect.source.binding.is_some_and(|source| {
                            source.entity == entity && self.binding(entity) == Some(source)
                        }))
            }) || match &side.cast {
                CastState::Idle => false,
                CastState::Casting(cast) => {
                    *id == entity
                        || (self.hostile_skill_v5(cast.skill.id)
                            && cast.targets.iter().any(|target| {
                                target.entity == entity && self.binding(entity) == Some(*target)
                            }))
                },
            }
        })
    }

    fn hostile_skill_v5(&self, skill: crate::domain::zone::skill::SkillId) -> bool {
        self.phase3
            .as_ref()
            .and_then(|phase| phase.canonical.rules.skills().get(skill))
            .is_some_and(hostile_skill)
    }

    fn prepare_live_learn(
        &self,
        id: EntityId,
        account: AccountId,
        generation: SessionGeneration,
        skill: SkillRef,
    ) -> Result<PreparedLearning, RejectReason> {
        let player = self.class_player(id, account, generation)?;
        let phase = self
            .phase3
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?;
        let e = self.entities.get(&id).ok_or(RejectReason::UnknownEntity)?;
        let c = e.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let side = phase
            .canonical
            .entities
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let mutation = prepare_learn(
            &player.class_state,
            side,
            LearnContext {
                character: CharacterId::from_uuid(id.as_uuid()),
                identity: &player.identity,
                classes: self.classes.as_deref().ok_or(RejectReason::InvalidLoad)?,
                rules: &phase.canonical.rules,
                level: c.sheet.level(),
                alive: !e.targeting.dead,
                engaged: self.engaged_for_learning(id),
                position: e.pos,
            },
            skill,
            phase.canonical.next_install_sequence,
            self.phase4.is_some(),
        )
        .map_err(learn_reject)?;
        // Derive and narrow every fallible resource/speed projection before publishing any
        // ledger/allocation mutation. Only this entity and its sidecar are cloned.
        let mut entity = e.clone();
        let mut sidecar = mutation.phase3;
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut entity
            .combat
            .as_mut()
            .ok_or(RejectReason::InvalidLoad)?
            .role
        else {
            return Err(RejectReason::InvalidLoad);
        };
        player.class_state = mutation.ledger;
        let effective = self
            .effective_for(&entity, &sidecar)
            .map_err(|_| RejectReason::InvalidLoad)?;
        entity.speed = effective
            .movement_speed()
            .map_err(|_| RejectReason::InvalidLoad)?;
        let combat = entity.combat.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let CombatRole::Player {
            progression: Some(player),
            ..
        } = &mut combat.role
        else {
            return Err(RejectReason::InvalidLoad);
        };
        player.max_cp = effective.max_cp;
        player.class_state.cp = player.class_state.cp.min(player.max_cp);
        combat.hp = combat.hp.min(effective.physical.max_hp());
        combat.mp = combat.mp.min(effective.physical.max_mp());
        combat.sheet = effective.physical;
        if combat.hp == combat.sheet.max_hp() {
            sidecar.fractions.hp = 0;
        }
        if combat.mp == combat.sheet.max_mp() {
            sidecar.fractions.mp = 0;
        }
        if player.class_state.cp == player.max_cp {
            sidecar.fractions.cp = 0;
        }
        Ok(PreparedLearning {
            entity,
            sidecar,
            next_sequence: mutation.next_modifier_sequence,
            spent: mutation.sp_spent,
            required_items: mutation.required_items,
        })
    }

    /// V5 transfer is a command transaction: retained handlers never replay onStart,
    /// paid ranks survive metadata grants, and resource ratios use final effective maxima.
    pub(super) fn transfer_v5(
        &mut self,
        tick: Tick,
        command: &AppliedCommand,
        staged: &StagedActions,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let ZoneCommand::ChangeClass {
            entity,
            account,
            request_key,
            target,
        } = command.command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        let actor = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let player = self.class_player(entity, account, actor.generation)?;
        if player.class_state.receipt(request_key).is_some() {
            events.extend(self.change_class(tick, entity, account, request_key, target)?);
            return Ok(());
        }
        if self.engaged_for_learning(entity) || self.staged_engaged_v5(entity, staged)
            || events.iter().any(|e| matches!(e, ZoneEvent::SkillLearned { entity: id, .. } | ZoneEvent::ClassTransfer { entity: id, .. } if *id == entity))
        { return Err(RejectReason::InCombat); }
        if let Some(reason) = self.transfer_unmet(entity, target).first() {
            return Err(*reason);
        }
        let c = actor.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let old = (
            c.hp,
            c.sheet.max_hp(),
            c.mp,
            c.sheet.max_mp(),
            player.class_state.cp,
            player.max_cp,
        );
        let old_speed = actor.speed;
        let mut candidate = self.clone();
        let mut changes = Vec::new();
        let removed: Vec<_> = candidate
            .sidecar(entity)
            .map_err(|_| RejectReason::InvalidLoad)?
            .effects
            .entries()
            .values()
            .filter(|e| !e.flags.stay_on_subclass_change)
            .map(|e| e.id)
            .collect();
        for effect in removed {
            candidate
                .remove_effect_v5(
                    tick,
                    entity,
                    effect,
                    crate::domain::zone::effects::EffectRemoveReason::Transfer,
                    &mut changes,
                )
                .map_err(|_| RejectReason::InvalidLoad)?;
        }
        changes.extend(candidate.change_class(tick, entity, account, request_key, target)?);
        candidate
            .sync_passives_v5(entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        candidate
            .rebuild_v5(entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let e = candidate
            .entities
            .get_mut(&entity)
            .ok_or(RejectReason::InvalidLoad)?;
        let c = e.combat.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &mut c.role
        else {
            return Err(RejectReason::InvalidLoad);
        };
        c.hp = crate::domain::zone::transfer_resource(old.0, old.1, c.sheet.max_hp(), 1);
        c.mp = crate::domain::zone::transfer_resource(old.2, old.3, c.sheet.max_mp(), 0);
        p.class_state.cp = crate::domain::zone::transfer_resource(old.4, old.5, p.max_cp, 0);
        if e.speed != old_speed {
            changes.push(ZoneEvent::EntityMove {
                tick,
                entity,
                pos: e.pos,
                dest: e.dest,
                speed: e.speed,
            });
        }
        candidate
            .sidecar_mut(entity)
            .map_err(|_| RejectReason::InvalidLoad)?
            .fractions = ResourceFractions::default();
        candidate
            .freeze_transfers_v5(&mut changes)
            .map_err(|_| RejectReason::InvalidLoad)?;
        *self = candidate;
        events.extend(changes);
        Ok(())
    }

    fn staged_engaged_v5(&self, entity: EntityId, staged: &StagedActions) -> bool {
        staged.pending_attacks().any(|(caster, target)| {
            self.binding(caster.entity) == Some(caster)
                && (caster.entity == entity
                    || target.is_some_and(|t| {
                        t.entity == entity
                            && self
                                .binding(entity)
                                .is_some_and(|b| t.incarnation == Some(b.incarnation))
                    }))
        }) || staged.pending().any(|entry| {
            (entry.caster.entity == entity && self.binding(entity) == Some(entry.caster))
                || (self.hostile_skill_v5(entry.skill_id)
                    && self.binding(entry.caster.entity) == Some(entry.caster)
                    && entry.target.is_some_and(|t| {
                        t.entity == entity
                            && self
                                .binding(entity)
                                .is_some_and(|b| t.incarnation == Some(b.incarnation))
                    }))
        }) || staged.pending_item_uses().any(|entry| {
            if self.binding(entry.caster.entity) != Some(entry.caster) {
                return false;
            }
            if entry.caster.entity == entity {
                return true;
            }
            let crate::domain::zone::ItemCommandKind::Use { instance, .. } = entry.command.kind
            else {
                return false;
            };
            let hostile = self
                .phase4
                .as_ref()
                .and_then(|items| {
                    items
                        .canonical
                        .players
                        .get(&entry.caster.entity)
                        .and_then(|player| player.inventory.get(&instance))
                        .and_then(|item| items.rules.parts().items.get(&item.template_id))
                })
                .is_some_and(|item| {
                    item.skills.iter().any(|skill| {
                        self.hostile_skill_v5(crate::domain::zone::skill::SkillId(skill.id))
                    })
                });
            hostile
                && entry.timing.target.is_some_and(|target| {
                    target.entity == entity
                        && self
                            .binding(entity)
                            .is_some_and(|binding| target.incarnation == Some(binding.incarnation))
                })
        })
    }

    /// Freeze only newly successful receipts, after the tick's final resource/stat barriers.
    /// Previously frozen retries never change with later gameplay or reconnects.
    pub(super) fn freeze_transfers_v5(
        &mut self,
        events: &mut [ZoneEvent],
    ) -> Result<(), TickError> {
        for event in events {
            let ZoneEvent::ClassTransfer {
                entity, receipt, ..
            } = event
            else {
                continue;
            };
            let e = self.entities.get_mut(entity).ok_or_else(missing)?;
            let c = e.combat.as_mut().ok_or_else(missing)?;
            let CombatRole::Player {
                xp,
                progression: Some(p),
                ..
            } = &mut c.role
            else {
                return Err(missing());
            };
            let result = &mut receipt.result;
            result.level = c.sheet.level();
            result.xp = *xp;
            result.sp = p.class_state.sp;
            result.hp = c.hp;
            result.max_hp = c.sheet.max_hp();
            result.mp = c.mp;
            result.max_mp = c.sheet.max_mp();
            result.cp = p.class_state.cp;
            result.max_cp = p.max_cp;
            result.position_millitiles = [e.pos.x.raw(), e.pos.y.raw()];
            result.token_tier_1_count = p.class_state.token_tier_1_count;
            result.token_tier_2_count = p.class_state.token_tier_2_count;
            let stored = p
                .class_state
                .successful_transfer_receipts
                .iter_mut()
                .find(|r| r.key == receipt.key)
                .ok_or_else(missing)?;
            stored.result.clone_from(result);
        }
        Ok(())
    }

    pub(super) fn learn_v5(
        &mut self,
        tick: Tick,
        command: &AppliedCommand,
        staged: &StagedActions,
        initial_revision: Option<u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let ZoneCommand::LearnSkill {
            entity,
            account,
            request_key,
            skill_id,
            skill_level,
        } = command.command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        let CommandSource::Session {
            entity: source,
            generation,
        } = command.source
        else {
            return Err(RejectReason::NotPermitted);
        };
        if source != entity {
            return Err(RejectReason::NotPermitted);
        }
        // The application serializes this already; refusing a malformed recorded draft
        // prevents two exact-rank successes from sharing one final receipt projection.
        if events.iter().any(
            |e| matches!(e, ZoneEvent::SkillLearned { entity: learned, .. } | ZoneEvent::ClassTransfer { entity: learned, .. } if *learned == entity),
        ) {
            return Err(RejectReason::InCombat);
        }
        if self.staged_engaged_v5(entity, staged) {
            return Err(RejectReason::InCombat);
        }
        let skill = SkillRef {
            id: skill_id,
            rank: skill_level,
        };
        let prepared = self.prepare_live_learn(entity, account, generation, skill)?;
        if prepared.required_items.is_empty() {
            return self.commit_learn_v5(
                tick,
                entity,
                account,
                request_key,
                skill,
                prepared,
                events,
            );
        }
        // V6 skill-book learning (E3.4/D47) is one transaction on a candidate: pay the
        // book(s) through the ledger, re-derive the learn on the paid inventory, learn the
        // rank, then publish the owner's checkpoint. Any refusal leaves no partial effect.
        self.learn_items_owned(entity, &prepared.required_items)?;
        let mut candidate = self.clone();
        let previous = events.iter().find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(projection) = event {
                (projection.entity == entity).then(|| projection.as_ref().clone())
            } else {
                None
            }
        });
        let mut projection = candidate.pay_learn_items_v6(
            tick,
            entity,
            request_key,
            skill,
            &prepared.required_items,
            previous,
            initial_revision,
        )?;
        let prepared = candidate.prepare_live_learn(entity, account, generation, skill)?;
        let mut local = Vec::new();
        candidate.commit_learn_v5(
            tick,
            entity,
            account,
            request_key,
            skill,
            prepared,
            &mut local,
        )?;
        let phase = &candidate
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical;
        projection.checkpoint = candidate
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(entity, tick, phase)
            .map_err(|_| RejectReason::InvalidLoad)?;
        projection
            .validate_staged(
                phase
                    .checkpoint(entity, tick)
                    .map_err(|_| RejectReason::InvalidLoad)?
                    .modifier_install_order
                    .len(),
            )
            .map_err(|_| RejectReason::InvalidLoad)?;
        *self = candidate;
        events.retain(|event| !matches!(event, ZoneEvent::ItemCheckpoint(p) if p.entity == entity));
        events.extend(local);
        events.push(ZoneEvent::ItemCheckpoint(Box::new(projection)));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // The command identity travels with the prepared learn.
    fn commit_learn_v5(
        &mut self,
        tick: Tick,
        entity: EntityId,
        account: AccountId,
        request_key: uuid::Uuid,
        skill: SkillRef,
        prepared: PreparedLearning,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let old = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let old_combat = old.combat.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let new_combat = prepared
            .entity
            .combat
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?;
        let vitals_changed = old_combat.hp != new_combat.hp
            || old_combat.sheet.max_hp() != new_combat.sheet.max_hp();
        let speed_changed = old.speed != prepared.entity.speed;
        // Seed the fact from the fully staged entity; end-of-tick replaces resources with
        // the final post-callback state. No key enters the canonical state or its digest.
        let result = self
            .learn_result_v5(tick, &prepared.entity, &prepared.sidecar, skill, prepared.spent)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let phase = self.phase3.as_mut().ok_or(RejectReason::InvalidLoad)?;
        phase.canonical.next_install_sequence = prepared.next_sequence;
        phase.canonical.entities.insert(entity, prepared.sidecar);
        phase.effective.remove(&entity);
        phase.schedule_dirty.insert(entity);
        if vitals_changed {
            phase.vitals_dirty.insert(entity);
        }
        if speed_changed {
            events.push(ZoneEvent::EntityMove {
                tick,
                entity,
                pos: prepared.entity.pos,
                dest: prepared.entity.dest,
                speed: prepared.entity.speed,
            });
        }
        self.entities.insert(entity, prepared.entity);
        events.push(ZoneEvent::SkillLearned {
            tick,
            entity,
            account,
            request_key,
            result: Box::new(result),
        });
        Ok(())
    }

    /// E3.4/D47 skill-book payment: take each required template (lowest owned instance IDs
    /// first) through one D69 Consume aggregate per instance, in the D66/D94 ledger shape.
    /// The cause is the learn receipt, in the API settlement contract: family
    /// `skill_learn_payment` with `receipt` (the `LearnSkill` request key), `skill_id`,
    /// `skill_level` and `count_delta`. Opens the owner's staged projection; the caller
    /// publishes it once the rank is learned. D92 `learned_item_cost` carries one fact on the
    /// commit tick (evidence only; one fact when the row has one book).
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // One mutation, its evidence and ledger rows.
    fn pay_learn_items_v6(
        &mut self,
        tick: Tick,
        id: EntityId,
        request_key: uuid::Uuid,
        skill: SkillRef,
        required: &[(u32, u32)],
        previous: Option<crate::domain::zone::Phase4Projection>,
        initial_revision: Option<u64>,
    ) -> Result<crate::domain::zone::Phase4Projection, RejectReason> {
        use crate::domain::zone::item_inventory::InventoryEdit;
        use crate::domain::zone::phase4_projection::{
            ItemAuditEvidence, ItemEvidenceValue as V, ItemOperation,
        };
        use crate::domain::zone::Phase4Projection;
        let subject = self.binding(id).ok_or(RejectReason::InvalidLoad)?;
        let items = self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        // T37 (D79): one step per owner-tick, measured against the tick-start revision.
        let changed = initial_revision.map_or_else(
            || previous.as_ref().is_some_and(|p| !p.ledger.is_empty()),
            |initial| initial != player.inventory_revision,
        );
        let revision = if changed {
            player.inventory_revision
        } else {
            player
                .inventory_revision
                .checked_add(1)
                .filter(|r| i64::try_from(*r).is_ok())
                .ok_or(RejectReason::InvalidLoad)?
        };
        let edits = required
            .iter()
            .map(|(template, count)| InventoryEdit::TakeTemplate {
                template: *template,
                count: u64::from(*count),
                preferred: None,
            })
            .collect::<Vec<_>>();
        let payment = InventoryMutation::prepare(
            &player.inventory,
            items.canonical.allocator,
            &edits,
            &items.rules,
            InventoryLimits::from_player(player),
            InventoryBoundary {
                tick,
                revision,
                origin: crate::domain::zone::ItemCreationOrigin {
                    zone: self.seed.zone,
                    epoch: self.seed.epoch,
                },
            },
        )
        .map_err(|_| RejectReason::SkillItemRequired)?;
        if payment.changes.is_empty() {
            return Err(RejectReason::InvalidLoad);
        }
        for change in &payment.changes {
            self.guard_item_removal_v6(
                id,
                change.before.as_ref().ok_or(RejectReason::InvalidLoad)?,
            )?;
        }
        let resolution = format!("learn:{request_key}");
        // D118: prepare_learn refuses multi-template ranks, so every payment has its fact.
        let [(template, count)] = required else {
            return Err(RejectReason::SkillDeferred);
        };
        if self.items_evidence_enabled() {
            let records = |inventory: &BTreeMap<
                crate::domain::zone::ItemInstanceId,
                crate::domain::zone::ItemInstance,
            >| {
                inventory
                    .values()
                    .filter(|item| item.template_id == *template && item.count > 0)
                    .map(|item| {
                        serde_json::json!({"instance_id": item.id.get(),
                            "template_id": item.template_id, "count": item.count})
                    })
                    .collect::<Vec<_>>()
            };
            let mut consumed = serde_json::Map::new();
            for change in &payment.changes {
                if let Some(before) = &change.before {
                    let take = before
                        .count
                        .saturating_sub(change.after.as_ref().map_or(0, |i| i.count));
                    consumed.insert(before.id.get().to_string(), serde_json::json!(take));
                }
            }
            let inputs = serde_json::json!({"skill_id": skill.id.0, "rank": skill.rank,
                "consume_id": template, "consume_count": count,
                "inventory": records(&player.inventory), "phase": "complete", "repeat": false});
            let actual = serde_json::json!({"accepted": true,
                "inventory": records(&payment.inventory), "consumed": consumed,
                "completion_consumed": {}, "repeated_consumed": {}, "phase": "complete",
                "draws": []});
            self.item_formula_v6(id, "learned_item_cost", inputs, actual, &resolution, None);
        }
        let account = match &self
            .entities
            .get(&id)
            .and_then(|e| e.combat.as_ref())
            .ok_or(RejectReason::InvalidLoad)?
            .role
        {
            CombatRole::Player {
                progression: Some(p),
                ..
            } => p.identity.account_id,
            CombatRole::Player {
                progression: None, ..
            }
            | CombatRole::Npc { .. } => return Err(RejectReason::NotAPlayer),
        };
        let items = self.phase4.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let player = items
            .canonical
            .players
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        player.inventory = payment.inventory;
        player.inventory_revision = revision;
        items.canonical.allocator = payment.allocator;
        for change in &payment.changes {
            self.item_evidence_delta(subject, change, ItemOperation::Consume, 0, None, &resolution);
        }
        // The checkpoint is replaced after the rank is learned; this one only seeds a new
        // projection with the paid inventory.
        let checkpoint = self
            .phase4
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .checkpoint(
                id,
                tick,
                &self
                    .phase3
                    .as_ref()
                    .ok_or(RejectReason::InvalidLoad)?
                    .canonical,
            )
            .map_err(|_| RejectReason::InvalidLoad)?;
        let mut projection = previous.unwrap_or_else(|| Phase4Projection {
            enchant_attempts: Vec::new(),
            exchanges: Vec::new(),
            refines: Vec::new(),
            paused_core: None,
            tick,
            entity: id,
            account,
            checkpoint: checkpoint.clone(),
            ledger: Vec::new(),
            receipts: Vec::new(),
            materializations: Vec::new(),
            evidence: Vec::new(),
        });
        projection.checkpoint = checkpoint;
        for change in payment.changes {
            let before = change.before.as_ref().ok_or(RejectReason::InvalidLoad)?;
            let count_delta = i64::try_from(
                before
                    .count
                    .checked_sub(change.after.as_ref().map_or(0, |i| i.count))
                    .ok_or(RejectReason::InvalidLoad)?,
            )
            .ok()
            .and_then(i64::checked_neg)
            .ok_or(RejectReason::InvalidLoad)?;
            let cause = ItemAuditEvidence {
                family: "skill_learn_payment".into(),
                fields: BTreeMap::from([
                    ("receipt".into(), V::Text(request_key.to_string())),
                    ("skill_id".into(), V::Unsigned(u64::from(skill.id.0))),
                    ("skill_level".into(), V::Unsigned(u64::from(skill.rank))),
                    ("count_delta".into(), V::Signed(count_delta)),
                ]),
            };
            super::item_commands::append_item_consumption(
                &mut projection,
                change,
                count_delta,
                cause,
            )?;
        }
        Ok(projection)
    }

    fn learn_result_v5(
        &self,
        tick: Tick,
        entity: &Entity,
        sidecar: &Phase3Entity,
        skill: SkillRef,
        spent: u64,
    ) -> Result<FrozenLearnResult, TickError> {
        let c = entity.combat.as_ref().ok_or_else(missing)?;
        let CombatRole::Player {
            progression: Some(p),
            ..
        } = &c.role
        else {
            return Err(missing());
        };
        let stats = self.effective_for(entity, sidecar).map_err(invariant)?;
        Ok(FrozenLearnResult {
            character_id: CharacterId::from_uuid(entity.id.as_uuid()),
            skill,
            sp_spent: spent,
            sp_remaining: p.class_state.sp,
            tick,
            hp: c.hp,
            max_hp: c.sheet.max_hp(),
            mp: c.mp,
            max_mp: c.sheet.max_mp(),
            cp: p.class_state.cp,
            max_cp: p.max_cp,
            effective_stats: LearnedEffectiveStats {
                p_atk_q: stats.physical.p_atk().raw(),
                p_def_q: stats.physical.p_def().raw(),
                m_atk_q: stats.m_atk_q,
                m_def_q: stats.m_def_q,
                attack_speed: u32::try_from(
                    stats
                        .physical
                        .attack_speed()
                        .raw()
                        .checked_div(Q)
                        .ok_or_else(missing)?,
                )
                .map_err(invariant)?,
                cast_speed: stats.cast_speed,
                speed_q: stats.run_speed_q,
                accuracy_q: stats.physical.accuracy().raw(),
                evasion_q: stats.physical.evasion().raw(),
                crit_permille: stats.physical.crit_permille(),
            },
            incarnation: c.incarnation,
        })
    }

    pub(super) fn freeze_learning_v5(
        &self,
        tick: Tick,
        events: &mut [ZoneEvent],
    ) -> Result<(), TickError> {
        for event in events {
            if let ZoneEvent::SkillLearned { entity, result, .. } = event {
                **result = self.learn_result_v5(
                    tick,
                    self.entities.get(entity).ok_or_else(missing)?,
                    self.sidecar(*entity)?,
                    result.skill,
                    result.sp_spent,
                )?;
            }
        }
        Ok(())
    }
}
