//! Live V5 life transitions at the shared recipient barrier. The legacy transition
//! supplies XP/progression and target cleanup; the sidecar supplies effects and penalty.
use super::{
    effects, invariant, missing, Arc, CastCancelReason, CastState, CombatRole, DamageRequest,
    DamageRoll, DamageRollSite, EffectList, EffectSourceKind, Entity, EntityId, EntityKind,
    RegenSchedule, RejectReason, ResourceFractions, Tick, TickError, ZoneEvent, ZoneState,
};
use crate::domain::zone::state_death::{death_penalty_roll_required, prepare_death, DeathContext};

impl ZoneState {
    pub(super) fn expire_policy_v5(
        &mut self,
        tick: Tick,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), TickError> {
        let phase = self.phase3.as_mut().ok_or_else(missing)?;
        for (id, side) in &mut phase.canonical.entities {
            if let Some(player) = side.player.as_mut() {
                player.pvp.expire_flag(tick);
            }
            if let Some(offer) = side.expire_resurrect_offer(tick) {
                events.push(offer_event(
                    tick,
                    *id,
                    &offer,
                    crate::domain::zone::ResurrectOfferState::Expired,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn note_hostility_v5(
        &mut self,
        tick: Tick,
        source: effects::TargetBinding,
        target: effects::TargetBinding,
    ) -> Result<(), TickError> {
        if !self.complete_runtime_v5()
            || self.binding(source.entity) != Some(source)
            || self.binding(target.entity) != Some(target)
        {
            return Ok(());
        }
        let Some(victim) = self
            .sidecar(target.entity)?
            .player
            .as_ref()
            .map(|p| crate::domain::zone::state_pvp::VictimPvp::at(&p.pvp, tick))
        else {
            return Ok(());
        };
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        if let Some(player) = self.sidecar_mut(source.entity)?.player.as_mut() {
            player
                .pvp
                .note_hostile_act(victim, tick, rules.pvp())
                .map_err(invariant)?;
        }
        Ok(())
    }

    pub(super) fn resurrect_offer_v5(
        &mut self,
        tick: Tick,
        caster: effects::TargetBinding,
        target: effects::TargetBinding,
        skill: crate::domain::zone::skill::SkillRef,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<crate::domain::zone::SkillTargetResult, TickError> {
        use crate::domain::zone::state_death::{
            prepare_resurrect_offer, OfferContext, OfferMutation,
        };
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        let (_, wit) = self.base_property_v5(caster.entity, "WIT")?;
        let offer = prepare_resurrect_offer(
            self.sidecar(target.entity)?,
            OfferContext {
                now: tick,
                caster,
                target_incarnation: target.incarnation,
                target_dead: self
                    .entities
                    .get(&target.entity)
                    .is_some_and(|e| e.targeting.dead),
                resurrection_blocked: false,
                skill,
                wit_bonus: wit,
            },
            &rules,
            self.phase3
                .as_ref()
                .ok_or_else(missing)?
                .canonical
                .next_offer_id,
        )
        .map_err(invariant)?;
        if let OfferMutation::Created {
            offer,
            next_offer_id,
        } = offer
        {
            self.phase3
                .as_mut()
                .ok_or_else(missing)?
                .canonical
                .next_offer_id = next_offer_id;
            self.sidecar_mut(target.entity)?
                .player
                .as_mut()
                .ok_or_else(missing)?
                .resurrect_offer = Some(offer.clone());
            events.push(offer_event(
                tick,
                target.entity,
                &offer,
                crate::domain::zone::ResurrectOfferState::Offered,
            ));
        }
        Ok(crate::domain::zone::SkillTargetResult {
            target,
            resolution_kind: crate::domain::zone::SkillResolutionKind::Resurrection,
            outcome: crate::domain::zone::SkillOutcome::Hit,
            rolled: 0,
            cp_absorbed: 0,
            hp_removed: 0,
            heal: 0,
            target_hp_after: 0,
            applied_effect_ids: Vec::new(),
        })
    }

    pub(super) fn accept_resurrect_v5(
        &mut self,
        tick: Tick,
        entity: EntityId,
        offer_id: crate::domain::zone::OfferId,
        accept: bool,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        use crate::domain::zone::state_death::{
            prepare_accept_resurrect, AcceptContext, DeathError,
        };
        let mut candidate = self.clone();
        let binding = candidate
            .binding(entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let e = candidate
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let c = e.combat.as_ref().ok_or(RejectReason::NotAPlayer)?;
        let CombatRole::Player { xp, .. } = c.role else {
            return Err(RejectReason::NotAPlayer);
        };
        let rules = &candidate
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .rules;
        let mutation = prepare_accept_resurrect(
            candidate
                .sidecar(entity)
                .map_err(|_| RejectReason::InvalidLoad)?,
            AcceptContext {
                now: tick,
                incarnation: binding.incarnation,
                dead: e.targeting.dead,
                resurrection_blocked: false,
                offer_id,
                accept,
                current_xp: xp,
            },
            rules,
        )
        .map_err(|error| match error {
            DeathError::NoOffer => RejectReason::NoResurrectOffer,
            DeathError::Blocked => RejectReason::InvalidTargetType,
            DeathError::InvalidState
            | DeathError::Exhausted
            | DeathError::Effect(_)
            | DeathError::Math(_) => RejectReason::InvalidLoad,
        })?;
        let mut events = vec![offer_event(
            tick,
            entity,
            &mutation.offer,
            if accept {
                crate::domain::zone::ResurrectOfferState::Accepted
            } else {
                crate::domain::zone::ResurrectOfferState::Declined
            },
        )];
        *candidate
            .sidecar_mut(entity)
            .map_err(|_| RejectReason::InvalidLoad)? = mutation.phase3;
        if accept {
            candidate.note_progress(entity);
            let e = candidate
                .entities
                .get_mut(&entity)
                .ok_or(RejectReason::UnknownEntity)?;
            e.targeting.dead = false;
            e.dest = None;
            let c = e.combat.as_mut().ok_or(RejectReason::InvalidLoad)?;
            if let CombatRole::Player { xp, .. } = &mut c.role {
                *xp = mutation.xp_after;
            }
            c.incarnation = mutation.incarnation;
            c.protected_until = None;
            c.swing = None;
            c.auto_attack = false;
            c.chasing = false;
            candidate.relevel(entity);
            candidate
                .sync_passives_v5(entity)
                .map_err(|_| RejectReason::InvalidLoad)?;
            candidate
                .rebuild_v5(entity)
                .map_err(|_| RejectReason::InvalidLoad)?;
            let e = candidate
                .entities
                .get_mut(&entity)
                .ok_or(RejectReason::UnknownEntity)?;
            let c = e.combat.as_mut().ok_or(RejectReason::InvalidLoad)?;
            let (hp, mp) = crate::domain::zone::town_respawn_vitals(
                candidate
                    .rules
                    .as_ref()
                    .ok_or(RejectReason::InvalidLoad)?
                    .constants(),
                &c.sheet,
            )
            .map_err(|_| RejectReason::InvalidLoad)?;
            c.hp = hp;
            c.mp = c.mp.max(mp);
            // Resurrection preserves current CP, restarts regen and has no town protection.
            events.push(ZoneEvent::EntityRespawned {
                tick,
                entity,
                position: e.pos,
                hp,
                incarnation: c.incarnation,
            });
            if let Some(note) = candidate.progress.get_mut(&entity) {
                note.respawned = true;
            }
        }
        *self = candidate;
        Ok(events)
    }

    /// Command transaction: carry retained modifiers into the new life, clear this
    /// death's refundable loss, and restart fractional regeneration at the town boundary.
    pub(super) fn respawn_town_v5(
        &mut self,
        tick: Tick,
        entity: EntityId,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let mut candidate = self.clone();
        let binding = candidate
            .binding(entity)
            .ok_or(RejectReason::UnknownEntity)?;
        let incarnation = binding
            .incarnation
            .checked_add(1)
            .ok_or(RejectReason::InvalidLoad)?;
        let side = candidate
            .sidecar(entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        let mut side = side.clone();
        let rules = &candidate
            .phase3
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .canonical
            .rules;
        side.player
            .as_mut()
            .ok_or(RejectReason::NotAPlayer)?
            .last_death = None;
        side.cast = CastState::Idle;
        side.fractions = ResourceFractions::default();
        side.regen = RegenSchedule::start(tick, true, rules.regen())
            .map_err(|_| RejectReason::InvalidLoad)?;
        side.effects = EffectList::from_entries(
            side.effects
                .entries()
                .iter()
                .map(|(id, effect)| {
                    let mut effect = effect.clone();
                    effect.target_incarnation = incarnation;
                    (*id, effect)
                })
                .collect(),
        )
        .map_err(|_| RejectReason::InvalidLoad)?;
        let invalidated = side.invalidate_resurrect_offer();
        let mut events = candidate.respawn_player(tick, entity)?;
        if let Some(offer) = invalidated {
            events.insert(
                0,
                offer_event(
                    tick,
                    entity,
                    &offer,
                    crate::domain::zone::ResurrectOfferState::Expired,
                ),
            );
        }
        *candidate
            .sidecar_mut(entity)
            .map_err(|_| RejectReason::InvalidLoad)? = side;
        candidate
            .rebuild_v5(entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        *self = candidate;
        Ok(events)
    }

    #[allow(clippy::too_many_lines)] // Death, rewards and policy are ordered at one recipient barrier.
    pub(super) fn finish_death_v5(
        &mut self,
        tick: Tick,
        request: DamageRequest,
        recipient: &Entity,
        events: &mut Vec<ZoneEvent>,
        draw: &mut DamageRoll<'_>,
    ) -> Result<(), TickError> {
        let target = request.target;
        let source = request.source.binding.ok_or_else(missing)?;
        let c = recipient.combat.as_ref().ok_or_else(missing)?;
        let victim_pvp = self
            .sidecar(target.entity)?
            .player
            .as_ref()
            .map(|p| crate::domain::zone::state_pvp::VictimPvp::at(&p.pvp, tick));
        let source_before = self
            .entities
            .get(&source.entity)
            .filter(|e| !e.targeting.dead && self.binding(source.entity) == Some(source))
            .and_then(|e| e.combat.as_ref())
            .and_then(|c| match c.role {
                CombatRole::Player { xp, .. } => Some((xp, c.sheet.level())),
                CombatRole::Npc { .. } => None,
            });
        let pre_death_xp = match c.role {
            CombatRole::Player { xp, .. } => xp,
            CombatRole::Npc { .. } => 0,
        };
        let xp_lost = if recipient.kind == EntityKind::Player {
            crate::domain::zone::progression::death_xp_loss(
                self.rules.as_deref().ok_or_else(missing)?,
                c.sheet.level(),
            )
            .map_err(invariant)?
            .min(pre_death_xp)
        } else {
            0
        };
        let rules = Arc::clone(&self.phase3.as_ref().ok_or_else(missing)?.canonical.rules);
        let context = DeathContext {
            incarnation: target.incarnation,
            was_alive: !recipient.targeting.dead && c.hp > 0,
            killer_kind: Some(match request.source.kind {
                EffectSourceKind::Player => EntityKind::Player,
                EffectSourceKind::Npc => EntityKind::Npc,
                EffectSourceKind::System => return Err(missing()),
            }),
            inside_peace: rules
                .geometry()
                .peace_square(self.bounds)
                .map_err(invariant)?
                .contains(recipient.pos),
            pre_death_xp,
            xp_lost,
        };
        let roll = if death_penalty_roll_required(self.sidecar(target.entity)?, context)
            .map_err(invariant)?
        {
            Some(draw(self, DamageRollSite::DeathPenalty, target, 100).map_err(invariant)?)
        } else {
            None
        };
        self.cancel_v5(tick, target.entity, None, CastCancelReason::CasterDied, events);
        let death = prepare_death(
            self.sidecar(target.entity)?,
            context,
            &rules,
            roll,
            self.phase3
                .as_ref()
                .ok_or_else(missing)?
                .canonical
                .next_install_sequence,
        )
        .map_err(invariant)?;
        self.phase3
            .as_mut()
            .ok_or_else(missing)?
            .canonical
            .next_install_sequence = death.next_install_sequence;
        *self.sidecar_mut(target.entity)? = death.phase3;
        for old in death.removed_effects {
            events.push(ZoneEvent::EffectRemoved {
                tick,
                target,
                effect_id: old.id,
                reason: effects::EffectRemoveReason::Death,
            });
        }
        // The legacy cleanup charges exactly the forecast loss and relevels once. Rebuild
        // AFTER it, because its profession sheet does not include V5 penalty/effects.
        self.kill(
            tick,
            target.entity,
            Some(super::super::combat_phase::Blow {
                attacker: source.entity,
            }),
            events,
        );
        if let Some((xp_before, level_before)) = source_before {
            if self.complete_runtime_v5() {
                let xp_after = self
                    .entities
                    .get(&source.entity)
                    .and_then(|e| e.combat.as_ref())
                    .and_then(|c| match c.role {
                        CombatRole::Player { xp, .. } => Some(xp),
                        CombatRole::Npc { .. } => None,
                    })
                    .ok_or_else(missing)?;
                if let Some(player) = self.sidecar_mut(source.entity)?.player.as_mut() {
                    if let Some(victim) = victim_pvp {
                        player.pvp.apply_player_kill(victim);
                    } else {
                        player
                            .pvp
                            .decay_karma(
                                xp_after.saturating_sub(xp_before),
                                level_before,
                                rules.pvp(),
                            )
                            .map_err(invariant)?;
                    }
                }
            }
        }
        if recipient.kind == EntityKind::Player {
            self.sync_passives_v5(target.entity)?;
        }
        self.rebuild_v5(target.entity)?;
        if recipient.kind == EntityKind::Npc && request.source.kind == EffectSourceKind::Player {
            self.credit_sp_v5(tick, source, recipient, events)?;
            self.sync_passives_v5(source.entity)?;
            self.rebuild_v5(source.entity)?;
        }
        Ok(())
    }
}

pub(super) fn offer_event(
    tick: Tick,
    entity: EntityId,
    offer: &crate::domain::zone::ResurrectOffer,
    state: crate::domain::zone::ResurrectOfferState,
) -> ZoneEvent {
    ZoneEvent::ResurrectOffer {
        tick,
        target: effects::TargetBinding {
            entity,
            incarnation: offer.target_incarnation,
        },
        offer_id: offer.id,
        caster: Some(offer.caster),
        restore_percent_q: offer.restore_percent_q,
        recoverable_xp: offer.recoverable_xp,
        expires_at: offer.expires_at,
        state,
    }
}
