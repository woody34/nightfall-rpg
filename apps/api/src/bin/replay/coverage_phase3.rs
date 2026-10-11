//! Schema-three lifecycle coverage from verified zone facts. Initial snapshots and
//! observer hydration are baselines, not starts; each container/cast identity ends once.
use std::collections::{BTreeMap, BTreeSet};

use nightfall_api::domain::zone::{
    effects::EffectId, CastId, CastState, EntityId, ZoneEvent, ZoneSnapshot,
};

use super::{increment, table, transition, Coverage, Transition};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct Stats {
    pub(super) cast_states: Vec<Transition>,
    pub(super) cast_end_reasons: BTreeMap<String, u64>,
    pub(super) effect_states: Vec<Transition>,
    pub(super) effect_remove_reasons: BTreeMap<String, u64>,
    pub(super) learning_commands: BTreeMap<String, u64>,
    pub(super) learning_committed: u64,
    pub(super) cp_absorption: BTreeMap<String, u64>,
}
impl Stats {
    pub(super) fn new() -> Self {
        Self {
            cast_states: table(&["Idle", "Casting"], &[("Idle", "Casting"), ("Casting", "Idle")]),
            cast_end_reasons: BTreeMap::new(),
            effect_states: table(
                &["Absent", "Active", "Removed"],
                &[("Absent", "Active"), ("Active", "Removed")],
            ),
            effect_remove_reasons: BTreeMap::new(),
            learning_commands: BTreeMap::new(),
            learning_committed: 0,
            cp_absorption: BTreeMap::new(),
        }
    }
}

#[derive(Default)]
pub(super) struct Tracker {
    casts: BTreeMap<EntityId, (u32, CastId)>,
    effects: BTreeSet<(EntityId, u32, EffectId)>,
}

impl Tracker {
    pub(super) fn baseline(snapshot: &ZoneSnapshot) -> Self {
        let mut tracker = Self::default();
        if let Some(phase) = &snapshot.phase3 {
            for (id, side) in &phase.entities {
                if let CastState::Casting(cast) = &side.cast {
                    tracker
                        .casts
                        .insert(*id, (cast.caster_incarnation, cast.id));
                }
                tracker.effects.extend(
                    side.effects
                        .entries()
                        .values()
                        .map(|effect| (*id, effect.target_incarnation, effect.id)),
                );
            }
        }
        tracker
    }

    fn end_cast(
        &mut self,
        report: &mut Stats,
        entity: EntityId,
        identity: (u32, CastId),
        reason: String,
    ) {
        if self.casts.get(&entity) == Some(&identity) {
            self.casts.remove(&entity);
            transition(&mut report.cast_states, "Casting", "Idle");
            increment(&mut report.cast_end_reasons, reason);
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)] // Other verified facts do not change these lifecycles.
    pub(super) fn facts(&mut self, report: &mut Coverage, events: &[ZoneEvent]) {
        let Some(report) = &mut report.phase3 else {
            return;
        };
        for event in events {
            match event {
                ZoneEvent::SkillCastStarted {
                    caster,
                    cast_id,
                    baseline: None,
                    ..
                } => {
                    let identity = (caster.incarnation, *cast_id);
                    if self.casts.insert(caster.entity, identity) != Some(identity) {
                        transition(&mut report.cast_states, "Idle", "Casting");
                    }
                },
                ZoneEvent::SkillCastCancelled {
                    caster,
                    cast_id,
                    reason,
                    ..
                } => {
                    self.end_cast(
                        report,
                        caster.entity,
                        (caster.incarnation, *cast_id),
                        format!("{reason:?}"),
                    );
                },
                ZoneEvent::SkillCastResult {
                    caster,
                    cast_id,
                    targets,
                    ..
                } => {
                    self.end_cast(
                        report,
                        caster.entity,
                        (caster.incarnation, *cast_id),
                        "Completed".into(),
                    );
                    for target in targets.iter().filter(|target| target.cp_absorbed > 0) {
                        increment(&mut report.cp_absorption, "skill_resolutions".into());
                        let whole = report
                            .cp_absorption
                            .entry("whole_removed".into())
                            .or_default();
                        *whole = whole.saturating_add(u64::from(target.cp_absorbed));
                    }
                },
                ZoneEvent::EffectApplied {
                    target,
                    effect_id,
                    baseline: None,
                    ..
                } => {
                    if self
                        .effects
                        .insert((target.entity, target.incarnation, *effect_id))
                    {
                        transition(&mut report.effect_states, "Absent", "Active");
                    }
                },
                ZoneEvent::EffectRemoved {
                    target,
                    effect_id,
                    reason,
                    ..
                } => {
                    if self
                        .effects
                        .remove(&(target.entity, target.incarnation, *effect_id))
                    {
                        transition(&mut report.effect_states, "Active", "Removed");
                        increment(&mut report.effect_remove_reasons, format!("{reason:?}"));
                    }
                },
                ZoneEvent::SkillLearned { .. } => {
                    report.learning_committed = report.learning_committed.saturating_add(1);
                },
                ZoneEvent::EntityDespawn { entity, .. } => {
                    if let Some(identity) = self.casts.get(entity).copied() {
                        self.end_cast(report, *entity, identity, "Despawned".into());
                    }
                    // Departure ends our observation of these identities. Do not invent
                    // EffectRemoved facts or an expiry/death reason for this boundary.
                    self.effects.retain(|(id, _, _)| id != entity);
                },
                _ => {},
            }
        }
    }
}
