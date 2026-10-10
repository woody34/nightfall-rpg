//! Deterministic Phase 3 work selection. These are phase-local keys, not stored queues:
//! every callback revalidates its live container/life after earlier work has run. Keeping
//! them derived avoids stale canonical timer indexes in snapshots and digests.

use super::effects::{EffectError, EffectId};
use super::{CastId, CastState, EntityId, EntityOrdinal, Phase3Snapshot, Tick};

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
