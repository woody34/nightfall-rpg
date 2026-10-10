//! Bounded Phase 3 instruments. Gameplay producers count committed zone-wide facts once;
//! owner/AOI copies and hydration must never be counted as transitions.

use super::Metrics;
use crate::application::AppError;
use opentelemetry::KeyValue;
use std::time::Duration;

macro_rules! labels {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(missing_docs)] // Finite labels are documented by their literal names.
        pub enum $name { $($variant),+ }
        impl $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];
            const fn as_str(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
        }
    };
}
labels! {
    /// Admitted cast lifecycle, excluding ingress refusals and bootstrap hydration.
    SkillCastOutcome { Started => "started", Completed => "completed", Cancelled => "cancelled" }
}
labels! {
    /// Resolution family; never a skill id or name.
    SkillResolutionKind { Physical => "physical", Magical => "magical", Restoration => "restoration", Effect => "effect", Resurrection => "resurrection" }
}
labels! {
    /// Bounded target resolution classification.
    SkillOutcome { Applied => "applied", Evaded => "evaded", Resisted => "resisted", Half => "half", Critical => "critical" }
}
labels! {
    /// Supported handler families. Removed containers use the same immutable handler kind.
    SkillEffectKind {
        PhysicalAttack => "physical_attack", MagicalAttack => "magical_attack", MagicalAttackRange => "magical_attack_range",
        Stun => "stun", Sleep => "sleep", Root => "root", Buff => "buff", Debuff => "debuff",
        TickHp => "tick_hp", TickMp => "tick_mp", Heal => "heal", HpPerMax => "hp_per_max", MaxHp => "max_hp", MaxMp => "max_mp",
        Mp => "mp", Cp => "cp", Resurrection => "resurrection", GetAgro => "get_agro", AddHate => "add_hate",
        DispelBySlot => "dispel_by_slot", DispelByCategory => "dispel_by_category", DefenceTrait => "defence_trait", AttackTrait => "attack_trait", Passive => "passive"
    }
}
labels! {
    /// Effect transitions; excludes restore and AOI hydration.
    SkillEffectChange { Applied => "applied", Removed => "removed" }
}
labels! {
    /// Authenticated RPC attempts, including frozen receipt replay and rollout refusal.
    SkillLearnOutcome { Success => "success", Rejected => "rejected", Unavailable => "unavailable", Internal => "internal" }
}
impl SkillLearnOutcome {
    /// No dynamic error text or caller identifiers become labels.
    pub const fn from_result<T>(result: &Result<T, AppError>) -> Self {
        match result {
            Ok(_) => Self::Success,
            Err(AppError::Unavailable(_)) => Self::Unavailable,
            Err(AppError::Infrastructure(_)) => Self::Internal,
            Err(
                AppError::InvalidArgument(_)
                | AppError::NotFound { .. }
                | AppError::AlreadyExists(_)
                | AppError::Unauthenticated(_)
                | AppError::PermissionDenied(_)
                | AppError::IdempotencyConflict
                | AppError::FailedPrecondition(_)
                | AppError::ResourceExhausted(_),
            ) => Self::Rejected,
        }
    }
}

impl Metrics {
    pub(super) fn initialize_phase3(&self) {
        for outcome in SkillCastOutcome::ALL {
            self.skill_casts_total
                .add(0, &[KeyValue::new("outcome", outcome.as_str())]);
        }
        for kind in SkillResolutionKind::ALL {
            for outcome in SkillOutcome::ALL {
                self.skill_resolutions_total.add(
                    0,
                    &[
                        KeyValue::new("kind", kind.as_str()),
                        KeyValue::new("outcome", outcome.as_str()),
                    ],
                );
            }
        }
        for kind in SkillEffectKind::ALL {
            for change in SkillEffectChange::ALL {
                self.skill_effects_total.add(
                    0,
                    &[
                        KeyValue::new("kind", kind.as_str()),
                        KeyValue::new("change", change.as_str()),
                    ],
                );
            }
        }
        for outcome in SkillLearnOutcome::ALL {
            self.skill_learn_requests_total
                .add(0, &[KeyValue::new("outcome", outcome.as_str())]);
        }
        self.skills_learned_total.add(0, &[]);
    }
    /// Call once per admitted cast fact, after durability barriers; never for AOI copies.
    pub fn record_skill_cast(&self, outcome: SkillCastOutcome) {
        self.skill_casts_total
            .add(1, &[KeyValue::new("outcome", outcome.as_str())]);
    }
    /// Call once per admitted target outcome, before per-observer filtering.
    pub fn record_skill_resolution(&self, kind: SkillResolutionKind, outcome: SkillOutcome) {
        self.skill_resolutions_total.add(
            1,
            &[
                KeyValue::new("kind", kind.as_str()),
                KeyValue::new("outcome", outcome.as_str()),
            ],
        );
    }
    /// Count installed/removed handler instances after commit; restore/hydration is excluded.
    pub fn record_skill_effect(&self, kind: SkillEffectKind, change: SkillEffectChange) {
        self.skill_effects_total.add(
            1,
            &[
                KeyValue::new("kind", kind.as_str()),
                KeyValue::new("change", change.as_str()),
            ],
        );
    }
    /// Authenticated endpoint attempts, including receipt replays; not a purchase counter.
    pub fn record_skill_learn_request(&self, latency: Duration, outcome: SkillLearnOutcome) {
        let labels = [KeyValue::new("outcome", outcome.as_str())];
        self.skill_learn_requests_total.add(1, &labels);
        self.skill_learn_seconds
            .record(latency.as_secs_f64(), &labels);
    }
    /// Applied checkpoint acknowledgements only. Replayed receipts/checkpoints are excluded.
    pub fn record_skill_learn_committed(&self) {
        self.skills_learned_total.add(1, &[]);
    }
}
