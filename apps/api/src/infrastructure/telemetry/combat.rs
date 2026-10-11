//! Live-only admitted combat facts. Entity ids are lookup keys, never metric labels.

use std::collections::BTreeMap;
use std::time::Duration;

use opentelemetry::KeyValue;

use super::Metrics;
use crate::application::zone_actor::{TickStats, TickTelemetry, ZoneTelemetry};
use crate::domain::zone::{
    AppliedTick, AttackOutcome, EntityId, EntityKind, Intention, ZoneEvent, ZoneSnapshot,
};

/// Fixed E3.2 intention label set (one per [`Intention`]). No string-valued API can create
/// unbounded series.
#[derive(Debug, Clone, Copy)]
pub enum NpcIntention {
    /// No current activity.
    Idle,
    /// Thinking or wandering.
    Active,
    /// Fighting a target.
    Attack,
    /// Leashed back to the spawn home.
    ReturnHome,
    /// Waiting for corpse removal or respawn.
    Dead,
}

impl From<Intention> for NpcIntention {
    fn from(i: Intention) -> Self {
        match i {
            Intention::Idle => Self::Idle,
            Intention::Active => Self::Active,
            Intention::Attack => Self::Attack,
            Intention::ReturnHome => Self::ReturnHome,
            Intention::Dead => Self::Dead,
        }
    }
}

impl NpcIntention {
    const ALL: [Self; 5] = [
        Self::Idle,
        Self::Active,
        Self::Attack,
        Self::ReturnHome,
        Self::Dead,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Attack => "attack",
            Self::ReturnHome => "return_home",
            Self::Dead => "dead",
        }
    }
}

impl Metrics {
    pub(super) fn initialize_combat(&self) {
        for outcome in ["miss", "hit", "crit"] {
            self.combat_attacks_total
                .add(0, &[KeyValue::new("outcome", outcome)]);
        }
        for kind in ["player", "npc"] {
            self.combat_deaths_total
                .add(0, &[KeyValue::new("kind", kind)]);
            self.combat_respawns_total
                .add(0, &[KeyValue::new("kind", kind)]);
        }
        self.combat_levelups_total.add(0, &[]);
        self.combat_xp_gained_total.add(0, &[]);
        for intention in NpcIntention::ALL {
            self.npc_intention_transitions_total
                .add(0, &[KeyValue::new("to", intention.label())]);
        }
    }

    /// One admitted `NpcIntentionChanged`. Never counts AI decisions before durable admission.
    pub fn record_npc_intention_transition(&self, to: NpcIntention) {
        self.npc_intention_transitions_total
            .add(1, &[KeyValue::new("to", to.label())]);
    }
}

impl ZoneTelemetry for Metrics {
    fn consumer(&self, initial: &ZoneSnapshot) -> Box<dyn TickTelemetry> {
        Box::new(CombatConsumer {
            metrics: self.clone(),
            kinds: initial.entities.iter().map(|e| (e.id, e.kind)).collect(),
            rules: initial.phase3.as_ref().map(|phase| phase.rules.clone()),
            effects: initial
                .phase3
                .as_ref()
                .map(|phase| {
                    phase
                        .entities
                        .iter()
                        .flat_map(|(entity, side)| {
                            side.effects
                                .entries()
                                .iter()
                                .map(|(id, effect)| ((*entity, *id), effect.skill.id))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

struct CombatConsumer {
    metrics: Metrics,
    // Bounded by live zone population; removed on actual zone despawn, never AOI exit.
    kinds: BTreeMap<EntityId, EntityKind>,
    rules: Option<std::sync::Arc<crate::domain::zone::Phase3Rules>>,
    effects: BTreeMap<
        (EntityId, crate::domain::zone::effects::EffectId),
        crate::domain::zone::skill::SkillId,
    >,
}

impl CombatConsumer {
    fn record_effect(
        &self,
        skill: crate::domain::zone::skill::SkillId,
        change: super::SkillEffectChange,
    ) {
        let Some(definition) = self
            .rules
            .as_ref()
            .and_then(|rules| rules.skills().get(skill))
        else {
            return;
        };
        for handler in &definition.effects {
            if crate::domain::zone::effects::continuous_handler(&handler.name) {
                if let Some(kind) = super::SkillEffectKind::from_handler_name(&handler.name) {
                    self.metrics.record_skill_effect(kind, change);
                }
            }
        }
    }
    fn record_resolution_targets(&self, targets: &[crate::domain::zone::SkillTargetResult]) {
        for target in targets {
            use crate::domain::zone::{SkillOutcome as O, SkillResolutionKind as R};
            let kind = match target.resolution_kind {
                R::Physical => super::SkillResolutionKind::Physical,
                R::Magical => super::SkillResolutionKind::Magical,
                R::Effect => super::SkillResolutionKind::Effect,
                R::Heal => super::SkillResolutionKind::Restoration,
                R::Resurrection => super::SkillResolutionKind::Resurrection,
            };
            let outcome = match target.outcome {
                O::Hit => super::SkillOutcome::Applied,
                O::Miss => super::SkillOutcome::Evaded,
                O::Crit => super::SkillOutcome::Critical,
                O::Resist => super::SkillOutcome::Resisted,
                O::Half => super::SkillOutcome::Half,
            };
            self.metrics.record_skill_resolution(kind, outcome);
        }
    }
    fn index_hydrated_effects(&mut self, tick: &AppliedTick) {
        // Cold admission restores containers without an application fact. Hydration supplies
        // their new live IDs for later removal accounting, but never increments a counter.
        for outputs in tick.outputs.values() {
            for output in outputs {
                if let crate::domain::zone::ObserverOutput::Event(ZoneEvent::EffectApplied {
                    target,
                    effect_id,
                    skill,
                    baseline: Some(_),
                    ..
                }) = output
                {
                    if self.kinds.contains_key(&target.entity) {
                        self.effects.insert((target.entity, *effect_id), skill.id);
                    }
                }
            }
        }
    }
}

impl TickTelemetry for CombatConsumer {
    fn admitted(&mut self, tick: &AppliedTick) {
        for event in &tick.events {
            match event {
                ZoneEvent::EntitySpawn { entity, kind, .. } => {
                    self.kinds.insert(*entity, *kind);
                },
                ZoneEvent::EntityDespawn { entity, .. } => {
                    self.kinds.remove(entity);
                    self.effects.retain(|(target, _), _| target != entity);
                },
                ZoneEvent::AttackResult { outcome, .. } => {
                    let outcome = match outcome {
                        AttackOutcome::Miss => "miss",
                        AttackOutcome::Hit => "hit",
                        AttackOutcome::Crit => "crit",
                    };
                    self.metrics
                        .combat_attacks_total
                        .add(1, &[KeyValue::new("outcome", outcome)]);
                },
                ZoneEvent::EntityDied { entity, .. }
                | ZoneEvent::EntityRespawned { entity, .. } => {
                    if let Some(kind) = self.kinds.get(entity) {
                        let kind = match kind {
                            EntityKind::Player => "player",
                            EntityKind::Npc => "npc",
                        };
                        let counter = if matches!(event, ZoneEvent::EntityDied { .. }) {
                            &self.metrics.combat_deaths_total
                        } else {
                            &self.metrics.combat_respawns_total
                        };
                        counter.add(1, &[KeyValue::new("kind", kind)]);
                    } else {
                        // A broken subscription/index must not silently misclassify an NPC.
                        tracing::error!("combat telemetry missing entity kind");
                    }
                },
                ZoneEvent::XpGained { amount, .. } => {
                    self.metrics.combat_xp_gained_total.add(*amount, &[]);
                },
                ZoneEvent::LevelUp { .. } => self.metrics.combat_levelups_total.add(1, &[]),
                ZoneEvent::NpcIntentionChanged { to, .. } => {
                    self.metrics.record_npc_intention_transition((*to).into());
                },
                ZoneEvent::ClassTransfer {
                    old_class_id,
                    receipt,
                    ..
                } => {
                    // Only the internal success fact counts; public AOI fan-out is ignored.
                    self.metrics.class_transfers_total.add(
                        1,
                        &[
                            KeyValue::new("from", i64::from(old_class_id.0)),
                            KeyValue::new("to", i64::from(receipt.target_class_id.0)),
                        ],
                    );
                },
                ZoneEvent::SkillCastStarted { baseline, .. } => {
                    if baseline.is_none() {
                        self.metrics
                            .record_skill_cast(super::SkillCastOutcome::Started);
                    }
                },
                ZoneEvent::SkillCastCancelled { .. } => self
                    .metrics
                    .record_skill_cast(super::SkillCastOutcome::Cancelled),
                ZoneEvent::SkillCastResult { targets, .. } => {
                    self.metrics
                        .record_skill_cast(super::SkillCastOutcome::Completed);
                    self.record_resolution_targets(targets);
                },
                ZoneEvent::EffectApplied {
                    target,
                    effect_id,
                    skill,
                    baseline,
                    ..
                } => {
                    if baseline.is_none() {
                        self.record_effect(skill.id, super::SkillEffectChange::Applied);
                    }
                    self.effects.insert((target.entity, *effect_id), skill.id);
                },
                ZoneEvent::EffectRemoved {
                    target, effect_id, ..
                } => {
                    if let Some(skill) = self.effects.remove(&(target.entity, *effect_id)) {
                        self.record_effect(skill, super::SkillEffectChange::Removed);
                    }
                },
                ZoneEvent::PvpStateChanged { .. }
                | ZoneEvent::ResurrectOffer { .. }
                | ZoneEvent::SkillStateSync { .. }
                | ZoneEvent::SkillLearned { .. }
                | ZoneEvent::SpGained { .. }
                | ZoneEvent::SkillReuse { .. }
                | ZoneEvent::VitalsChanged { .. }
                | ZoneEvent::TokensReconciled { .. }
                | ZoneEvent::ClassChanged { .. }
                | ZoneEvent::AttackStarted { .. }
                | ZoneEvent::AttackCancelled { .. }
                | ZoneEvent::HateChanged { .. }
                | ZoneEvent::StatsChanged { .. }
                | ZoneEvent::TargetChanged { .. }
                | ZoneEvent::Progression(_)
                | ZoneEvent::EntityMove { .. } => {},
            }
        }
        self.index_hydrated_effects(tick);
    }

    fn stats(&mut self, stats: TickStats) {
        self.metrics
            .combat_tick_duration_seconds
            .record(Duration::from_micros(stats.duration_micros).as_secs_f64(), &[]);
    }
}

#[cfg(test)]
#[path = "combat_tests.rs"]
mod tests;

impl crate::application::checkpoint::CheckpointMetrics for Metrics {
    fn skill_learned(&self) {
        self.record_skill_learn_committed();
    }
    fn token_granted(&self, tier: u8, source: crate::domain::character_progression::TokenSource) {
        use crate::domain::character_progression::TokenSource;
        let tier = match tier {
            1 => "1",
            2 => "2",
            _ => return,
        };
        let source = match source {
            TokenSource::Admission => "admission",
            TokenSource::LevelUp => "level_up",
        };
        self.class_transfer_token_grants_total
            .add(1, &[KeyValue::new("tier", tier), KeyValue::new("source", source)]);
    }

    fn failed(&self) {
        self.record_checkpoint_failure();
    }
    fn lag(&self, age: Duration) {
        self.set_checkpoint_lag(age);
    }
}
