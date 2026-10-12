//! Revision-two projection. Identity and revival boundaries precede hydration; all
//! optional references are qualified by the final incarnation and observer visibility.
use super::*;
use crate::domain::zone::{
    AttackStartedPhase3, CombatViewPhase3, ResurrectOfferState, TargetChangedPhase3,
};

impl ZoneState {
    #[allow(clippy::too_many_lines, clippy::wildcard_enum_match_arm)]
    pub(super) fn observe_complete_v5(
        &mut self,
        tick: Tick,
        responses: BTreeMap<CommandSource, Vec<ObserverOutput>>,
        events: &[ZoneEvent],
    ) -> Result<BTreeMap<EntityId, Vec<ObserverOutput>>, TickError> {
        let mut outputs = self.observe(tick, responses, events);
        let revived: std::collections::BTreeSet<_> = events
            .iter()
            .filter_map(|e| {
                if let ZoneEvent::EntityRespawned {
                    entity,
                    incarnation,
                    ..
                } = e
                {
                    (self
                        .binding(*entity)
                        .is_some_and(|b| b.incarnation == *incarnation))
                    .then_some(*entity)
                } else {
                    None
                }
            })
            .collect();
        let selection_dirty: std::collections::BTreeSet<_> = events
            .iter()
            .filter_map(|e| {
                if let ZoneEvent::TargetChanged { entity, .. } = e {
                    Some(*entity)
                } else {
                    None
                }
            })
            .collect();
        // Only facts that require recursive reference/life filtering enter the per-observer
        // pass. Idle movement ticks retain the indexed legacy pass and avoid event scans.
        let filtered: Vec<_> = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ZoneEvent::EntityDied { .. }
                        | ZoneEvent::AttackStarted { .. }
                        | ZoneEvent::AttackResult { .. }
                        | ZoneEvent::EffectApplied { .. }
                        | ZoneEvent::EffectRemoved { .. }
                        | ZoneEvent::SkillCastStarted { .. }
                        | ZoneEvent::SkillCastResult { .. }
                        | ZoneEvent::SkillCastCancelled { .. }
                        | ZoneEvent::SkillLearned { .. }
                        | ZoneEvent::SkillReuse { .. }
                        | ZoneEvent::SpGained { .. }
                        | ZoneEvent::VitalsChanged { .. }
                        | ZoneEvent::PvpStateChanged { .. }
                        | ZoneEvent::ResurrectOffer { .. }
                )
            })
            .collect();
        for (observer, (_, known)) in &self.known {
            let sees = |id: EntityId| id == *observer || known.binary_search(&id).is_ok();
            let current = |binding: TargetBinding| {
                sees(binding.entity) && self.binding(binding.entity) == Some(binding)
            };
            let out = outputs.entry(*observer).or_default();
            let mut hydrate: std::collections::BTreeSet<_> = out
                .iter()
                .filter_map(|o| {
                    if let ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) = o {
                        Some(*entity)
                    } else {
                        None
                    }
                })
                .collect();
            hydrate.extend(revived.iter().filter(|id| sees(**id)).copied());
            let owner_baseline = hydrate.contains(observer);
            if hydrate.is_empty() && filtered.is_empty() && !selection_dirty.contains(observer) {
                continue;
            }
            let mut ordinary = Vec::new();
            let mut item_results = Vec::new();
            for mut output in std::mem::take(out) {
                match &mut output {
                    ObserverOutput::Accepted { .. } | ObserverOutput::Rejected(_) => {
                        out.push(output);
                    },
                    ObserverOutput::Event(ZoneEvent::EntitySpawn {
                        entity,
                        combat: Some(combat),
                        ..
                    }) => {
                        let pvp = self
                            .sidecar(*entity)
                            .ok()
                            .and_then(|s| s.player.as_ref())
                            .map(|p| &p.pvp);
                        combat.phase3 = Some(CombatViewPhase3 {
                            status_flags: self.effective(*entity)?.flags,
                            flag_until: pvp.and_then(|p| p.flag_until),
                            karma: pvp.map_or(0, |p| p.karma),
                        });
                        combat.swing = None;
                        out.push(output);
                    },
                    ObserverOutput::Event(
                        ZoneEvent::EntitySpawn { .. }
                        | ZoneEvent::EntityDespawn { .. }
                        | ZoneEvent::EntityRespawned { .. },
                    ) => out.push(output),
                    ObserverOutput::Event(
                        ZoneEvent::AttackStarted { .. }
                        | ZoneEvent::AttackResult { .. }
                        | ZoneEvent::TargetChanged { .. }
                        | ZoneEvent::EntityDied { .. },
                    ) => {},
                    ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. })
                        if owner_baseline && entity == observer => {},
                    ObserverOutput::Event(ZoneEvent::ItemView(view))
                        if owner_baseline && view.entity == *observer && view.owner_only() => {},
                    ObserverOutput::Event(ZoneEvent::ItemResult(_)) => item_results.push(output),
                    ObserverOutput::Event(_) => ordinary.push(output),
                }
            }
            if owner_baseline && self.binding(*observer).is_some() {
                out.push(ObserverOutput::Event(self.owner_stats_v5(
                    tick,
                    *observer,
                    Some(tick),
                )?));
                if self.phase4.is_none() {
                    out.push(ObserverOutput::Event(self.owner_sync_v5(tick, *observer)?));
                }
            }
            for id in &hydrate {
                let Some(binding) = self.binding(*id) else {
                    continue;
                };
                let side = self.sidecar(*id)?;
                if let Some(player) = &side.player {
                    out.push(ObserverOutput::Event(ZoneEvent::PvpStateChanged {
                        tick,
                        entity: *id,
                        incarnation: binding.incarnation,
                        pvp: player.pvp.clone(),
                        baseline: Some(tick),
                    }));
                    if id == observer {
                        if let Some(offer) = &player.resurrect_offer {
                            out.push(ObserverOutput::Event(ZoneEvent::ResurrectOffer {
                                tick,
                                target: binding,
                                offer_id: offer.id,
                                caster: Some(offer.caster).filter(|s| current(*s)),
                                restore_percent_q: offer.restore_percent_q,
                                recoverable_xp: offer.recoverable_xp,
                                expires_at: offer.expires_at,
                                state: ResurrectOfferState::Offered,
                            }));
                        }
                    }
                }
                for effect in side.effects.entries().values() {
                    let mut event = self.effect_applied_v5(tick, *id, effect, Some(tick))?;
                    if let ZoneEvent::EffectApplied { source, .. } = &mut event {
                        *source = source.filter(|s| current(*s));
                    }
                    out.push(ObserverOutput::Event(event));
                }
                if let CastState::Casting(cast) = &side.cast {
                    let mut event = cast_started(tick, *id, cast, Some(tick));
                    if let ZoneEvent::SkillCastStarted { target, .. } = &mut event {
                        *target = target.filter(|t| current(*t));
                    }
                    out.push(ObserverOutput::Event(event));
                }
                let combat = self
                    .entities
                    .get(id)
                    .and_then(|e| e.combat.as_ref())
                    .ok_or_else(missing)?;
                if let Some(swing) = combat.swing {
                    out.push(ObserverOutput::Event(ZoneEvent::AttackStarted {
                        tick: swing.start,
                        attacker: *id,
                        target: swing.target,
                        target_incarnation: swing.target_incarnation,
                        impact: swing.impact,
                        ready: swing.ready,
                        phase3: Some(AttackStartedPhase3 {
                            attacker_incarnation: binding.incarnation,
                            target_visible: current(TargetBinding {
                                entity: swing.target,
                                incarnation: swing.target_incarnation,
                            }),
                            baseline: Some(tick),
                        }),
                    }));
                }
                if revived.contains(id) {
                    out.push(ObserverOutput::Event(ZoneEvent::VitalsChanged {
                        tick,
                        entity: *id,
                        incarnation: binding.incarnation,
                        hp: combat.hp,
                        max_hp: combat.sheet.max_hp(),
                        baseline: Some(tick),
                    }));
                }
            }
            let selected = self.entities.get(observer).and_then(|e| e.targeting.target);
            // C17: retaining selection across AOI departure is an approved Nightfall deviation.
            // C2: a retained selection must follow its target's new identity, even when
            // the owner's identity and authoritative selection did not change.
            let selection_reentered = selected.is_some_and(|id| hydrate.contains(&id));
            if owner_baseline || selection_dirty.contains(observer) || selection_reentered {
                let target = self
                    .entities
                    .get(observer)
                    .and_then(|e| e.targeting.target)
                    .and_then(|id| self.binding(id))
                    .filter(|t| current(*t));
                out.push(ObserverOutput::Event(ZoneEvent::TargetChanged {
                    tick,
                    entity: *observer,
                    target: target.map(|t| t.entity),
                    phase3: Some(TargetChangedPhase3 {
                        target_incarnation: target.map(|t| t.incarnation),
                        baseline: owner_baseline.then_some(tick),
                    }),
                }));
            }
            if owner_baseline && self.phase4.is_some() && self.binding(*observer).is_some() {
                out.push(ObserverOutput::Event(self.owner_sync_v5(tick, *observer)?));
                if let Some(baseline) = self.item_baseline_v6(tick, *observer)? {
                    out.push(ObserverOutput::Event(baseline));
                }
            }
            out.extend(ordinary);
            for event in &filtered {
                let projected = match event {
                    ZoneEvent::EntityDied {
                        entity,
                        incarnation,
                        killer,
                        ..
                    } if current(TargetBinding {
                        entity: *entity,
                        incarnation: *incarnation,
                    }) && !hydrate.contains(entity) =>
                    {
                        let mut event = (*event).clone();
                        if let ZoneEvent::EntityDied {
                            killer: projected, ..
                        } = &mut event
                        {
                            *projected =
                                killer.filter(|id| sees(*id) && self.binding(*id).is_some());
                        }
                        Some(event)
                    },
                    ZoneEvent::EffectApplied { target, source, .. }
                        if current(*target) && !hydrate.contains(&target.entity) =>
                    {
                        let mut event = (*event).clone();
                        if let ZoneEvent::EffectApplied { source: s, .. } = &mut event {
                            *s = source.filter(|s| current(*s));
                        }
                        Some(event)
                    },
                    ZoneEvent::EffectRemoved { target, .. }
                        if current(*target) && !hydrate.contains(&target.entity) =>
                    {
                        Some((*event).clone())
                    },
                    ZoneEvent::SkillCastStarted { caster, target, .. }
                        if current(*caster) && !hydrate.contains(&caster.entity) =>
                    {
                        let mut event = (*event).clone();
                        if let ZoneEvent::SkillCastStarted { target: t, .. } = &mut event {
                            *t = target.filter(|t| current(*t));
                        }
                        Some(event)
                    },
                    ZoneEvent::SkillCastResult {
                        caster, targets, ..
                    } if current(*caster) && !hydrate.contains(&caster.entity) => {
                        let mut event = (*event).clone();
                        if let ZoneEvent::SkillCastResult { targets: t, .. } = &mut event {
                            *t = targets
                                .iter()
                                .filter(|row| {
                                    current(row.target) && !hydrate.contains(&row.target.entity)
                                })
                                .cloned()
                                .collect();
                        }
                        Some(event)
                    },
                    ZoneEvent::SkillCastCancelled { caster, .. }
                        if current(*caster) && !hydrate.contains(&caster.entity) =>
                    {
                        Some((*event).clone())
                    },
                    ZoneEvent::VitalsChanged {
                        entity,
                        incarnation,
                        ..
                    }
                    | ZoneEvent::PvpStateChanged {
                        entity,
                        incarnation,
                        ..
                    } if current(TargetBinding {
                        entity: *entity,
                        incarnation: *incarnation,
                    }) && !hydrate.contains(entity) =>
                    {
                        Some((*event).clone())
                    },
                    ZoneEvent::SkillLearned { entity, .. }
                    | ZoneEvent::SpGained { entity, .. }
                    | ZoneEvent::SkillReuse { entity, .. }
                        if entity == observer =>
                    {
                        Some((*event).clone())
                    },
                    ZoneEvent::AttackStarted {
                        attacker,
                        target,
                        target_incarnation,
                        phase3: Some(p),
                        ..
                    } if current(TargetBinding {
                        entity: *attacker,
                        incarnation: p.attacker_incarnation,
                    }) && !hydrate.contains(attacker) =>
                    {
                        let mut event = (*event).clone();
                        if let ZoneEvent::AttackStarted {
                            phase3: Some(p), ..
                        } = &mut event
                        {
                            p.target_visible = current(TargetBinding {
                                entity: *target,
                                incarnation: *target_incarnation,
                            });
                        }
                        Some(event)
                    },
                    ZoneEvent::AttackResult {
                        attacker,
                        target,
                        target_incarnation,
                        phase3: Some(p),
                        ..
                    } if current(TargetBinding {
                        entity: *attacker,
                        incarnation: p.attacker_incarnation,
                    }) && current(TargetBinding {
                        entity: *target,
                        incarnation: *target_incarnation,
                    }) && !hydrate.contains(attacker)
                        && !hydrate.contains(target) =>
                    {
                        Some((*event).clone())
                    },
                    ZoneEvent::ResurrectOffer {
                        target,
                        caster,
                        state,
                        ..
                    } if target.entity == *observer
                        && (current(*target)
                            || (revived.contains(observer)
                                && self.binding(*observer).is_some_and(|b| {
                                    Some(b.incarnation) == target.incarnation.checked_add(1)
                                })))
                        && (*state != ResurrectOfferState::Offered
                            || (!owner_baseline && current(*target))) =>
                    {
                        let mut event = (*event).clone();
                        if let ZoneEvent::ResurrectOffer { caster: c, .. } = &mut event {
                            *c = caster.filter(|c| current(*c));
                        }
                        Some(event)
                    },
                    _ => None,
                };
                if let Some(event) = projected {
                    out.push(ObserverOutput::Event(event));
                }
            }
            out.extend(item_results);
        }
        outputs.retain(|_, out| !out.is_empty());
        Ok(outputs)
    }
}
