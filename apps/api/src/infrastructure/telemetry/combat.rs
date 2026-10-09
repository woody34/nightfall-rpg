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
        })
    }
}

struct CombatConsumer {
    metrics: Metrics,
    // Bounded by live zone population; removed on actual zone despawn, never AOI exit.
    kinds: BTreeMap<EntityId, EntityKind>,
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
                ZoneEvent::AttackStarted { .. }
                | ZoneEvent::AttackCancelled { .. }
                | ZoneEvent::HateChanged { .. }
                | ZoneEvent::StatsChanged { .. }
                | ZoneEvent::TargetChanged { .. }
                | ZoneEvent::EntityMove { .. } => {},
            }
        }
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
