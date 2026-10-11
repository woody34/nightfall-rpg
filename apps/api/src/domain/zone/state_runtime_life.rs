//! Live V5 life transitions at the shared recipient barrier. The legacy transition
//! supplies XP/progression and target cleanup; the sidecar supplies effects and penalty.
use super::{
    effects, invariant, missing, Arc, CastCancelReason, CastState, CombatRole, DamageRequest,
    DamageRoll, DamageRollSite, EffectList, EffectSourceKind, Entity, EntityId, EntityKind,
    RegenSchedule, RejectReason, ResourceFractions, Tick, TickError, ZoneEvent, ZoneState,
};
use crate::domain::zone::state_death::{death_penalty_roll_required, prepare_death, DeathContext};

impl ZoneState {
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
        if side
            .player
            .as_ref()
            .is_some_and(|p| p.resurrect_offer.is_some())
        {
            // Coordinated ResurrectOffer terminal-event mapper has not landed yet.
            return Err(RejectReason::NotYetImplemented);
        }
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
        let events = candidate.respawn_player(tick, entity)?;
        *candidate
            .sidecar_mut(entity)
            .map_err(|_| RejectReason::InvalidLoad)? = side;
        candidate
            .rebuild_v5(entity)
            .map_err(|_| RejectReason::InvalidLoad)?;
        *self = candidate;
        Ok(events)
    }

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
