//! Deterministic Phase 3 work selection. These are phase-local keys, not stored queues:
//! every callback revalidates its live container/life after earlier work has run. Keeping
//! them derived avoids stale canonical timer indexes in snapshots and digests.

use super::effects::{EffectError, EffectId};
use super::{CastId, CastState, EntityId, EntityOrdinal, Phase3Entity, Phase3Snapshot, Tick};
use std::collections::{BTreeMap, BTreeSet};

/// Derived deadline indexes. Only due entries are visited at a tick boundary. Exact
/// reverse keys make removal/replacement bounded by one entity's live handlers; canceled
/// casts and removed effects leave no tombstones. None of these indexes are serialized.
///
/// The owner refreshes a changed entity before selecting the next phase. Candidate lists
/// remain phase-local: consumers revalidate each key against canonical state after earlier
/// callbacks, and refresh after advancing a pulse, launching/canceling a cast or death.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Phase3Schedule {
    entities: BTreeMap<EntityId, EntitySchedule>,
    expiries: BTreeSet<EffectExpiry>,
    pulses: BTreeSet<EffectPulse>,
    regen: BTreeSet<(Tick, EntityOrdinal, EntityId)>,
    launches: BTreeSet<CastWork>,
    impacts: BTreeSet<CastWork>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EntitySchedule {
    expiries: Vec<EffectExpiry>,
    pulses: Vec<EffectPulse>,
    regen: Option<(Tick, EntityOrdinal, EntityId)>,
    launch: Option<CastWork>,
    impact: Option<CastWork>,
}

impl EntitySchedule {
    fn build(target: EntityId, entity: &Phase3Entity) -> Result<Self, EffectError> {
        let mut keys = Self::default();
        for effect in entity.effects.entries().values() {
            if let Some(at) = effect.expires_at {
                keys.expiries.push(EffectExpiry {
                    at,
                    ordinal: entity.ordinal,
                    target,
                    effect: effect.id,
                });
            }
            for handler in &effect.handlers {
                if let Some(periodic) = &handler.periodic {
                    let at = periodic.schedule.due_tick()?;
                    // Expiry wins ties. There can never be another legal pulse for this
                    // handler if its next ideal callback already reaches the expiry.
                    if effect.expires_at.is_none_or(|until| at < until) {
                        keys.pulses.push(EffectPulse {
                            at,
                            ordinal: entity.ordinal,
                            target,
                            effect: effect.id,
                            handler_ordinal: handler.xml_ordinal,
                        });
                    }
                }
            }
        }
        keys.regen = entity.regen.next_at.map(|at| (at, entity.ordinal, target));
        if let CastState::Casting(cast) = &entity.cast {
            let work = CastWork {
                at: if cast.launched {
                    cast.complete_at
                } else {
                    cast.launch_at
                },
                ordinal: entity.ordinal,
                caster: target,
                cast: cast.id,
            };
            if cast.launched {
                keys.impact = Some(work);
            } else {
                keys.launch = Some(work);
            }
        }
        Ok(keys)
    }
}

impl Phase3Schedule {
    /// Reconstruct without invoking hooks, drawing RNG or changing canonical deadlines.
    pub fn rebuild(entities: &BTreeMap<EntityId, Phase3Entity>) -> Result<Self, EffectError> {
        let mut result = Self::default();
        for (id, entity) in entities {
            result.refresh(*id, Some(entity))?;
        }
        Ok(result)
    }

    /// Replace every derived key for one changed combatant, or remove a departed one.
    /// Validate new pulse arithmetic first so failure preserves the previous indexes.
    pub fn refresh(
        &mut self,
        id: EntityId,
        entity: Option<&Phase3Entity>,
    ) -> Result<(), EffectError> {
        let next = entity.map(|e| EntitySchedule::build(id, e)).transpose()?;
        if let Some(old) = self.entities.remove(&id) {
            for key in old.expiries {
                self.expiries.remove(&key);
            }
            for key in old.pulses {
                self.pulses.remove(&key);
            }
            if let Some(key) = old.regen {
                self.regen.remove(&key);
            }
            if let Some(key) = old.launch {
                self.launches.remove(&key);
            }
            if let Some(key) = old.impact {
                self.impacts.remove(&key);
            }
        }
        if let Some(next) = next {
            self.expiries.extend(next.expiries.iter().copied());
            self.pulses.extend(next.pulses.iter().copied());
            self.regen.extend(next.regen);
            self.launches.extend(next.launch);
            self.impacts.extend(next.impact);
            self.entities.insert(id, next);
        }
        Ok(())
    }

    /// Refresh only one recurring callback after its canonical schedule advances. Other
    /// timers are untouched. Whole-container mutations use `refresh` instead; this fast
    /// path requires the original scheduled callback and the same target allocation.
    pub fn refresh_pulse(
        &mut self,
        target: EntityId,
        effect: EffectId,
        handler_ordinal: u32,
        entity: &Phase3Entity,
    ) -> Result<(), EffectError> {
        let keys = self
            .entities
            .get_mut(&target)
            .ok_or(EffectError::Invalid("missing scheduled entity"))?;
        let index = keys
            .pulses
            .iter()
            .position(|key| {
                key.effect == effect
                    && key.handler_ordinal == handler_ordinal
                    && key.ordinal == entity.ordinal
            })
            .ok_or(EffectError::Invalid("missing scheduled callback"))?;
        let active = entity
            .effects
            .entries()
            .get(&effect)
            .ok_or(EffectError::Invalid("missing pulse container"))?;
        let periodic = active
            .handlers
            .iter()
            .find(|handler| handler.xml_ordinal == handler_ordinal)
            .and_then(|handler| handler.periodic.as_ref())
            .ok_or(EffectError::Invalid("missing periodic handler"))?;
        let at = periodic.schedule.due_tick()?;
        let next = active
            .expires_at
            .is_none_or(|until| at < until)
            .then_some(EffectPulse {
                at,
                ordinal: entity.ordinal,
                target,
                effect,
                handler_ordinal,
            });
        // All fallible work precedes removal. The reverse vector is bounded by the live
        // handler count and removal preserves its canonical container/XML order.
        let old = keys.pulses.remove(index);
        self.pulses.remove(&old);
        if let Some(next) = next {
            keys.pulses.insert(index, next);
            self.pulses.insert(next);
        }
        Ok(())
    }

    /// Expire these keys before querying pulses, refreshing each affected entity.
    pub fn effect_expiries(&self, now: Tick) -> Vec<EffectExpiry> {
        self.expiries
            .iter()
            .take_while(|key| key.at <= now)
            .copied()
            .collect()
    }

    /// Due callbacks after expiry removal. Consumers must still revalidate a returned key
    /// if an earlier callback removes/replaces its container in this same phase.
    pub fn effect_pulses(&self, now: Tick) -> Vec<EffectPulse> {
        self.pulses
            .iter()
            .take_while(|key| key.at <= now)
            .copied()
            .collect()
    }

    /// Regen selection examines due deadlines only, then applies entity-ordinal order.
    pub fn regen_candidates(&self, now: Tick) -> Vec<(EntityOrdinal, EntityId)> {
        let mut result: Vec<_> = self
            .regen
            .iter()
            .take_while(|(at, _, _)| *at <= now)
            .map(|(_, ordinal, id)| (*ordinal, *id))
            .collect();
        result.sort_unstable();
        result
    }

    /// Launch candidates, independent of completion deadlines.
    pub fn cast_launches(&self, now: Tick) -> Vec<CastWork> {
        self.launches
            .iter()
            .take_while(|key| key.at <= now)
            .copied()
            .collect()
    }

    /// Only launched casts have impact keys. Refresh after launch before this selection.
    pub fn cast_impacts(&self, now: Tick) -> Vec<CastWork> {
        self.impacts
            .iter()
            .take_while(|key| key.at <= now)
            .copied()
            .collect()
    }
}

/// Expiry ordering is deadline, target allocation order, target identity, effect identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub struct EffectExpiry {
    pub at: Tick,
    pub ordinal: EntityOrdinal,
    pub target: EntityId,
    pub effect: EffectId,
}

/// Pulse ordering additionally preserves the source handler XML order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub struct EffectPulse {
    pub at: Tick,
    pub ordinal: EntityOrdinal,
    pub target: EntityId,
    pub effect: EffectId,
    pub handler_ordinal: u32,
}

/// Launch and impact have separate ordered passes. A launch may be due at accepted start;
/// impacts cannot recursively join that start phase (L2Character.java:4577-4581).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub struct CastWork {
    pub at: Tick,
    pub ordinal: EntityOrdinal,
    pub caster: EntityId,
    pub cast: CastId,
}

impl Phase3Snapshot {
    /// Ordered expiry candidates at phase two. Removal precedes any pulse selection.
    pub fn effect_expiries(&self, now: Tick) -> Vec<EffectExpiry> {
        let mut due: Vec<_> = self
            .entities
            .iter()
            .flat_map(|(target, entity)| {
                entity
                    .effects
                    .expired(now)
                    .into_iter()
                    .map(|(at, effect)| EffectExpiry {
                        at,
                        ordinal: entity.ordinal,
                        target: *target,
                        effect,
                    })
            })
            .collect();
        due.sort_unstable();
        due
    }

    /// Ordered due periodic callbacks. Expired effects never enter this list, even if the
    /// caller inspects it before physically removing expiry candidates.
    pub fn effect_pulses(&self, now: Tick) -> Result<Vec<EffectPulse>, EffectError> {
        let mut due = Vec::new();
        for (target, entity) in &self.entities {
            for (at, effect, handler_ordinal) in entity.effects.pulses(now)? {
                due.push(EffectPulse {
                    at,
                    ordinal: entity.ordinal,
                    target: *target,
                    effect,
                    handler_ordinal,
                });
            }
        }
        due.sort_unstable();
        Ok(due)
    }

    /// Regen is processed after periodic effects, by entity ordinal then identity (design
    /// §3 phase three), not deadline order. Death removes the schedule immediately.
    pub fn regen_candidates(&self, now: Tick) -> Vec<(EntityOrdinal, EntityId)> {
        let mut due: Vec<_> = self
            .entities
            .iter()
            .filter(|(_, e)| e.regen.next_at.is_some_and(|at| at <= now))
            .map(|(id, e)| (e.ordinal, *id))
            .collect();
        due.sort_unstable();
        due
    }

    /// Cast launch candidates before the separate impact pass.
    pub fn cast_launches(&self, now: Tick) -> Vec<CastWork> {
        self.cast_work(now, true)
    }

    /// Impact candidates include only launched casts. Recompute after the launch pass;
    /// each consumer checks that earlier effects did not cancel the selected cast.
    pub fn cast_impacts(&self, now: Tick) -> Vec<CastWork> {
        self.cast_work(now, false)
    }

    fn cast_work(&self, now: Tick, launch: bool) -> Vec<CastWork> {
        let mut due = Vec::new();
        for (caster, entity) in &self.entities {
            if let CastState::Casting(cast) = &entity.cast {
                let at = if launch {
                    cast.launch_at
                } else {
                    cast.complete_at
                };
                if at <= now && cast.launched != launch {
                    due.push(CastWork {
                        at,
                        ordinal: entity.ordinal,
                        caster: *caster,
                        cast: cast.id,
                    });
                }
            }
        }
        due.sort_unstable();
        due
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::zone::effects::{EffectList, PulseSchedule};
    use crate::domain::zone::phase3::Phase3Zone;
    use crate::domain::zone::phase3_entity::tests::{blank, cast, source_effect};

    fn snapshot() -> Phase3Snapshot {
        Phase3Snapshot::new(
            crate::infrastructure::skill_data::load_phase3_rules(
                &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
                &[],
            )
            .unwrap(),
        )
    }
    fn id(value: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(value))
    }
    fn entity(snapshot: &Phase3Snapshot, ordinal: u64, effect_id: u64) -> Phase3Entity {
        let mut entity = blank();
        entity.ordinal = EntityOrdinal(ordinal);
        // Poison is a source-derived TickHp handler. Distinct target ordinals are reverse
        // UUID order so a UUID-first scheduler cannot accidentally pass these tests.
        let mut effect = source_effect(&snapshot.rules, effect_id, 1168);
        effect.expires_at = Some(Tick(21));
        let periodic = effect
            .handlers
            .iter_mut()
            .find_map(|h| h.periodic.as_mut())
            .unwrap();
        periodic.schedule = PulseSchedule::new(Tick(0), 666).unwrap();
        entity.effects = EffectList::from_entries(BTreeMap::from([(effect.id, effect)])).unwrap();
        entity.regen.next_at = Some(Tick(9));
        let mut flight = cast();
        flight.id = CastId(effect_id);
        entity.cast = CastState::Casting(flight);
        entity
    }
    fn compare(snapshot: &Phase3Snapshot, indexed: &Phase3Schedule, now: Tick) {
        assert_eq!(indexed.effect_expiries(now), snapshot.effect_expiries(now));
        assert_eq!(indexed.effect_pulses(now), snapshot.effect_pulses(now).unwrap());
        assert_eq!(indexed.regen_candidates(now), snapshot.regen_candidates(now));
        assert_eq!(indexed.cast_launches(now), snapshot.cast_launches(now));
        assert_eq!(indexed.cast_impacts(now), snapshot.cast_impacts(now));
    }

    #[test]
    fn indexed_order_and_restore_match_phase_local_canonical_selection() {
        let mut snapshot = snapshot();
        snapshot.entities.insert(id(90), entity(&snapshot, 1, 1));
        snapshot.entities.insert(id(10), entity(&snapshot, 2, 2));
        let mut indexed = Phase3Schedule::rebuild(&snapshot.entities).unwrap();
        for now in 0..=25 {
            let now = Tick(now);
            // The real phase order removes all expiry containers before selecting pulses.
            let expiries = indexed.effect_expiries(now);
            assert_eq!(expiries, snapshot.effect_expiries(now));
            for expiry in expiries {
                let entity = snapshot.entities.get_mut(&expiry.target).unwrap();
                entity.effects.remove(expiry.effect).unwrap();
                indexed.refresh(expiry.target, Some(entity)).unwrap();
            }
            compare(&snapshot, &indexed, now);
            for pulse in indexed.effect_pulses(now) {
                let entity = snapshot.entities.get_mut(&pulse.target).unwrap();
                entity
                    .effects
                    .advance_pulse(pulse.effect, pulse.handler_ordinal)
                    .unwrap();
                indexed
                    .refresh_pulse(pulse.target, pulse.effect, pulse.handler_ordinal, entity)
                    .unwrap();
            }
            for (_, target) in indexed.regen_candidates(now) {
                let entity = snapshot.entities.get_mut(&target).unwrap();
                entity.regen.take_due(now, snapshot.rules.regen()).unwrap();
                indexed.refresh(target, Some(entity)).unwrap();
            }
            let launches = indexed.cast_launches(now);
            if now == Tick(8) {
                assert_eq!(
                    launches.iter().map(|key| key.caster).collect::<Vec<_>>(),
                    [id(90), id(10)]
                );
            }
            for launch in launches {
                let entity = snapshot.entities.get_mut(&launch.caster).unwrap();
                if let CastState::Casting(cast) = &mut entity.cast {
                    cast.launched = true;
                }
                indexed.refresh(launch.caster, Some(entity)).unwrap();
            }
            assert_eq!(indexed.cast_impacts(now), snapshot.cast_impacts(now));
            for impact in indexed.cast_impacts(now) {
                let entity = snapshot.entities.get_mut(&impact.caster).unwrap();
                entity.cast = CastState::Idle;
                indexed.refresh(impact.caster, Some(entity)).unwrap();
            }
            // Canonical snapshot round trip, then the actual runtime-sidecar constructor.
            // This is index reconstruction, not a claim that this synthetic entity fixture
            // passes the separate full source/life/admission validators.
            let saved = serde_json::to_vec(&snapshot.entities).unwrap();
            let restored = serde_json::from_slice(&saved).unwrap();
            let restored_zone = Phase3Zone::from_canonical(Phase3Snapshot {
                entities: restored,
                ..snapshot.clone()
            })
            .unwrap();
            assert_eq!(restored_zone.schedule, indexed);
        }
    }

    #[test]
    fn replacement_removal_cancel_and_death_leave_no_stale_keys() {
        let snapshot = snapshot();
        let mut entity = entity(&snapshot, 1, 1);
        let mut indexed = Phase3Schedule::default();
        for generation in 1..=100 {
            entity.ordinal = EntityOrdinal(generation);
            if let CastState::Casting(cast) = &mut entity.cast {
                cast.id = CastId(generation);
                cast.launched = generation % 2 == 0;
            }
            indexed.refresh(id(1), Some(&entity)).unwrap();
            assert_eq!(indexed.entities.len(), 1);
            assert_eq!(indexed.expiries.len(), 1);
            assert_eq!(indexed.pulses.len(), 1);
            assert_eq!(indexed.regen.len(), 1);
            assert_eq!(indexed.launches.len() + indexed.impacts.len(), 1);
            assert!(indexed
                .effect_pulses(Tick(7))
                .iter()
                .all(|key| key.ordinal == entity.ordinal));
        }
        entity.cast = CastState::Idle;
        entity.regen.stop();
        entity.effects.remove(EffectId(1));
        indexed.refresh(id(1), Some(&entity)).unwrap();
        assert!(indexed.cast_launches(Tick(u64::MAX)).is_empty());
        assert!(indexed.cast_impacts(Tick(u64::MAX)).is_empty());
        assert!(indexed.effect_expiries(Tick(u64::MAX)).is_empty());
        assert!(indexed.effect_pulses(Tick(u64::MAX)).is_empty());
        assert!(indexed.regen_candidates(Tick(u64::MAX)).is_empty());
        indexed.refresh(id(1), None).unwrap();
        assert_eq!(indexed, Phase3Schedule::default());
    }

    #[test]
    fn expiry_wins_tie_and_regen_ignores_deadline_order() {
        let snapshot = snapshot();
        let mut first = entity(&snapshot, 1, 1);
        let mut second = entity(&snapshot, 2, 2);
        first.regen.next_at = Some(Tick(30));
        second.regen.next_at = Some(Tick(20));
        let mut effect = first.effects.remove(EffectId(1)).unwrap();
        effect.expires_at = Some(Tick(7));
        first.effects = EffectList::from_entries(BTreeMap::from([(effect.id, effect)])).unwrap();
        let indexed =
            Phase3Schedule::rebuild(&BTreeMap::from([(id(90), first.clone()), (id(10), second)]))
                .unwrap();
        assert_eq!(indexed.effect_pulses(Tick(7)).len(), 1);
        assert_eq!(indexed.effect_pulses(Tick(7))[0].target, id(10));
        assert_eq!(
            indexed.regen_candidates(Tick(30)),
            [(EntityOrdinal(1), id(90)), (EntityOrdinal(2), id(10))]
        );
    }
}
