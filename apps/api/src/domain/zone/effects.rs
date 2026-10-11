//! Canonical effect containers, periodic phase and paused-admission payloads (design §1.3).
//! Indexes are derived. Restoring a container never invokes handlers, healing or RNG.

use super::skill::{SkillId, SkillRef};
use super::skill_math::{periodic_amount, ExactRatio};
use super::{EntityId, StatError, Tick};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Zone-wide monotonically allocated effect identity. Zero is not a live effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectId(pub u64);

/// An entity reference fenced to one life. Observer projection hides the entire pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TargetBinding {
    pub entity: EntityId,
    pub incarnation: u32,
}

/// Frozen attribution survives source death/despawn without retaining a runtime pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectSourceKind {
    /// Player-created effect; hostility classification decides CP absorption separately.
    Player,
    /// NPC-created effect.
    Npc,
    /// Reconstructed system modifier, with no external entity reference.
    System,
}

/// Independent source identity and kind, including effects whose source has left the zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct EffectSource {
    pub binding: Option<TargetBinding>,
    pub kind: EffectSourceKind,
}

/// Each whole skill-wide container consumes one slot, regardless of child handler count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPool {
    /// Twenty plus clamped Enlarge contribution (zero through four).
    Buff,
    /// Shared twelve-slot song/dance pool.
    SongDance,
    /// Eight-slot debuff pool.
    Debuff,
    /// Passives/death penalty; never inserted in the finite timed effect list.
    System,
}

/// Validated exact frozen coefficient encoded as a reduced rational string, so JSON/SQL
/// do not lose i128 precision. This is an immutable input, not a second resource ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FrozenRatio(ExactRatio);

impl FrozenRatio {
    /// Preserve an already checked, reduced exact value.
    pub const fn new(value: ExactRatio) -> Self {
        Self(value)
    }
    /// Wider local arithmetic input; narrow only at the named source/runtime boundary.
    pub const fn value(self) -> ExactRatio {
        self.0
    }
}

impl From<FrozenRatio> for String {
    fn from(value: FrozenRatio) -> Self {
        let (numerator, denominator) = value.0.parts();
        format!("{numerator}/{denominator}")
    }
}
impl TryFrom<String> for FrozenRatio {
    type Error = StatError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let result = Self(ExactRatio::parse_literal(&value)?);
        if String::from(result) != value {
            return Err(StatError::Overflow);
        }
        Ok(result)
    }
}

/// Periodic resource; CP is an instant handler and is not a TickHp/TickMp resource kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodicResource {
    /// HP restoration or nonlethal DOT (clamp before CP classification/absorption).
    Hp,
    /// MP restoration or whole MP drain.
    Mp,
}

/// DIFF freezes its final source amount; PER reads the current whole+Q resource each pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum PeriodicPower {
    /// Already evaluated binary64 power × widened binary32 interval multiplier.
    Diff {
        /// Exact frozen source amount, including sign.
        amount: FrozenRatio,
    },
    /// No division by 100; preserve source multiplication and rounding order.
    Per {
        /// Source parameter before the binary64 product.
        power: FrozenRatio,
        /// Immutable widened binary32 interval multiplier.
        multiplier: FrozenRatio,
    },
}
impl PeriodicPower {
    /// Evaluate without drawing RNG or touching an entity.
    pub fn amount(self, current_q: i64) -> Result<ExactRatio, StatError> {
        match self {
            Self::Diff { amount } => Ok(amount.value()),
            Self::Per { power, multiplier } => {
                periodic_amount(power.value(), multiplier.value(), current_q, true)
            },
        }
    }
}

/// Source pulse phase: offsets are ideal milliseconds relative to an explicit anchor.
/// The anchor is application at first creation and actual admission when timers are rebased.
/// A restored pulse index retains its history; it is not used to reset the retained phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PulseSchedule {
    pub anchor: Tick,
    pub next_ideal_offset_ms: u64,
    pub pulse_index: u64,
    pub interval_ms: u32,
}
impl PulseSchedule {
    /// First periodic callback is after the source interval, never during onStart.
    pub fn new(applied: Tick, interval_ms: u32) -> Result<Self, EffectError> {
        let result = Self {
            anchor: applied,
            next_ideal_offset_ms: u64::from(interval_ms),
            pulse_index: 1,
            interval_ms,
        };
        result.due_tick()?;
        Ok(result)
    }
    /// Exact ceiling of the ideal offset; no accumulated tick-rounding drift.
    pub fn due_tick(&self) -> Result<Tick, EffectError> {
        if self.interval_ms == 0
            || !self.interval_ms.is_multiple_of(666)
            || self.interval_ms > i32::MAX.unsigned_abs()
            || self.pulse_index == 0
            || self.next_ideal_offset_ms == 0
        {
            return Err(EffectError::Invalid("invalid periodic phase"));
        }
        let ticks = self.next_ideal_offset_ms.div_ceil(100);
        Ok(Tick(
            self.anchor
                .0
                .checked_add(ticks)
                .ok_or(EffectError::Overflow)?,
        ))
    }
    /// Advance even when peace-zone or another eligibility gate suppresses a due pulse.
    /// Arithmetic is checked before either canonical field changes.
    pub fn advance(&mut self) -> Result<(), EffectError> {
        let offset = self
            .next_ideal_offset_ms
            .checked_add(u64::from(self.interval_ms))
            .ok_or(EffectError::Overflow)?;
        let index = self
            .pulse_index
            .checked_add(1)
            .ok_or(EffectError::Overflow)?;
        let next = Self {
            anchor: self.anchor,
            next_ideal_offset_ms: offset,
            pulse_index: index,
            interval_ms: self.interval_ms,
        };
        next.due_tick()?;
        *self = next;
        Ok(())
    }
    /// Remaining ideal phase relative to the checkpoint tick, preserving sub-tick offset.
    pub fn remaining_ms(&self, checkpoint: Tick) -> Result<u64, EffectError> {
        self.due_tick()?;
        let delta = i128::from(self.anchor.0)
            .checked_sub(i128::from(checkpoint.0))
            .and_then(|v| v.checked_mul(100))
            .and_then(|v| v.checked_add(i128::from(self.next_ideal_offset_ms)))
            .ok_or(EffectError::Overflow)?;
        let remaining = u64::try_from(delta)
            .map_err(|_| EffectError::Invalid("overdue periodic checkpoint"))?;
        if remaining == 0 {
            return Err(EffectError::Invalid("due periodic checkpoint"));
        }
        Ok(remaining)
    }
}

/// One already initialized handler. Generic/hook-owned Func descriptors are reconstructed
/// from its immutable skill/XML position; restoring this value never invokes onStart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ActiveHandler {
    pub xml_ordinal: u32,
    pub periodic: Option<ActivePeriodic>,
}

/// A live periodic handler's complete canonical state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ActivePeriodic {
    pub resource: PeriodicResource,
    pub power: PeriodicPower,
    pub schedule: PulseSchedule,
}

/// Source lifecycle flags, retained explicitly and checked against immutable skill data
/// by entity admission/restore. Removal of one provider never directly clears status bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
#[allow(clippy::struct_excessive_bools)] // Independent source flags, not mutually exclusive states.
pub struct EffectFlags {
    pub stay_after_death: bool,
    pub stay_on_subclass_change: bool,
    pub removed_on_damage: bool,
    pub removed_on_action_except_move: bool,
    pub irreplaceable: bool,
}

/// A whole live timed application. Entries and XML handler order are canonical; indexes
/// are rebuilt from these values. Passives and untimed death penalty are separate sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ActiveEffect {
    pub id: EffectId,
    pub skill: SkillRef,
    pub source: EffectSource,
    pub target_incarnation: u32,
    pub applied_at: Tick,
    pub expires_at: Option<Tick>,
    pub pool: EffectPool,
    pub abnormal_type: String,
    pub abnormal_level: u32,
    pub flags: EffectFlags,
    pub handlers: Vec<ActiveHandler>,
}

/// Global installation ordering is shared by passives, timed effects and the system
/// death-penalty modifier; saved effect references use retained vector indices, not old IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RetainedModifierSource {
    /// Reconstruct from the exact persisted learned rank.
    Passive {
        /// Learned source.
        skill: SkillRef,
    },
    /// Index into `Phase3Checkpoint.effect_remainders` (zero based).
    Effect {
        /// Retained whole-container index.
        effect_ordinal: u32,
    },
    /// Reconstruct skill5076 from the single `death_penalty_level` scalar.
    DeathPenalty,
}

/// Persisted pulse state, relative to checkpoint rather than a prior epoch's tick origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RetainedPeriodic {
    pub resource: PeriodicResource,
    pub power: PeriodicPower,
    pub interval_ms: u32,
    pub pulse_index: u64,
    pub next_in_ms: u64,
}

/// Persisted continuous handler; source order and typed payload are never untyped JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RetainedHandler {
    pub xml_ordinal: u32,
    pub periodic: Option<RetainedPeriodic>,
}

/// Paused finite application, retained only for `remaining_ticks` >100. The outer vector
/// must use old `(applied_at, EffectId)` order so fresh IDs preserve eviction order on admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RetainedEffect {
    pub skill: SkillRef,
    pub source: EffectSource,
    pub target_incarnation: u32,
    pub pool: EffectPool,
    pub abnormal_type: String,
    pub abnormal_level: u32,
    pub remaining_ticks: u64,
    pub flags: EffectFlags,
    pub handlers: Vec<RetainedHandler>,
}

/// Canonical effect or phase validation failure. Callers stage changes before committing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EffectError {
    /// A malformed identity, ordering, pool, source or deadline.
    #[error("invalid effect: {0}")]
    Invalid(&'static str),
    /// Checked deadline/index arithmetic exceeded its representation.
    #[error("effect arithmetic overflow")]
    Overflow,
}

/// Source stack identity excludes caster identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum StackKey {
    Abnormal(String),
    Skill(SkillId),
}
impl ActiveEffect {
    fn stack_key(&self) -> StackKey {
        if self.abnormal_type == "NONE" {
            StackKey::Skill(self.skill.id)
        } else {
            StackKey::Abnormal(self.abnormal_type.clone())
        }
    }
    fn validate(&self) -> Result<(), EffectError> {
        if self.id.0 == 0
            || self.skill.id.0 == 0
            || self.skill.rank == 0
            || self.abnormal_type.is_empty()
            || self.pool == EffectPool::System
            || self.abnormal_type == "SUMMON_CONDITION"
            || self
                .expires_at
                .is_some_and(|expiry| expiry <= self.applied_at)
            || (self.source.kind == EffectSourceKind::System) != self.source.binding.is_none()
        {
            return Err(EffectError::Invalid("identity, lifetime, pool or attribution"));
        }
        let mut last = None;
        for handler in &self.handlers {
            if last.is_some_and(|ordinal| ordinal >= handler.xml_ordinal) {
                return Err(EffectError::Invalid("handler XML order"));
            }
            last = Some(handler.xml_ordinal);
            if let Some(periodic) = &handler.periodic {
                periodic.schedule.due_tick()?;
            }
        }
        Ok(())
    }
}

/// Fixed finite pools from the accepted design; no caller-supplied arbitrary capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectCapacities {
    buff: usize,
}
impl EffectCapacities {
    /// Enlarge contribution is clamped to zero through four.
    pub fn for_enlarge(contribution: i32) -> Self {
        Self {
            buff: 20_usize.saturating_add(usize::try_from(contribution.clamp(0, 4)).unwrap_or(0)),
        }
    }
    fn limit(self, pool: EffectPool) -> usize {
        match pool {
            EffectPool::Buff => self.buff,
            EffectPool::SongDance => 12,
            EffectPool::Debuff => 8,
            EffectPool::System => 0,
        }
    }
}

/// Admission removes old entries before installing any new handler. The caller emits these
/// removals before application and executes onStart/generic Funcs in handler order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectAdmission {
    /// Lower abnormal level: leave existing application and timers unchanged.
    StrongerStack {
        /// Unchanged stronger application's identity.
        existing: EffectId,
    },
    /// Container admitted. Replacement precedes overflow removals in this vector.
    Applied {
        /// Full removed applications for deterministic cleanup and removal facts.
        removed: Vec<ActiveEffect>,
    },
}

/// Canonical entries plus rebuildable stack/age indexes. Only entries serialize or hash.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct EffectList {
    entries: BTreeMap<EffectId, ActiveEffect>,
    #[serde(skip)]
    stacks: BTreeMap<StackKey, EffectId>,
    #[serde(skip)]
    age: BTreeSet<(EffectPool, Tick, EffectId)>,
}
impl EffectList {
    /// Restore structural state without replaying any onStart hook, RNG or one-time heal.
    pub fn from_entries(entries: BTreeMap<EffectId, ActiveEffect>) -> Result<Self, EffectError> {
        let mut result = Self::default();
        for (id, effect) in entries {
            effect.validate()?;
            if id != effect.id || result.stacks.contains_key(&effect.stack_key()) {
                return Err(EffectError::Invalid("duplicate stack or inconsistent identity"));
            }
            result.insert(effect);
        }
        Ok(result)
    }
    /// Canonical `EffectId` order, independent of derived indexes.
    pub const fn entries(&self) -> &BTreeMap<EffectId, ActiveEffect> {
        &self.entries
    }
    /// Explicit check after rebuilding effective slot-capacity inputs.
    pub fn validate_capacity(&self, capacities: EffectCapacities) -> Result<(), EffectError> {
        for pool in [EffectPool::Buff, EffectPool::SongDance, EffectPool::Debuff] {
            if self.pool_count(pool) > capacities.limit(pool) {
                return Err(EffectError::Invalid("pool over capacity"));
            }
        }
        Ok(())
    }
    fn pool_count(&self, pool: EffectPool) -> usize {
        self.age
            .iter()
            .filter(|(entry_pool, _, _)| *entry_pool == pool)
            .count()
    }
    fn insert(&mut self, effect: ActiveEffect) {
        self.stacks.insert(effect.stack_key(), effect.id);
        self.age.insert((effect.pool, effect.applied_at, effect.id));
        self.entries.insert(effect.id, effect);
    }
    /// Remove exactly one application, updating all derived indexes together.
    pub fn remove(&mut self, id: EffectId) -> Option<ActiveEffect> {
        let effect = self.entries.remove(&id)?;
        self.stacks.remove(&effect.stack_key());
        self.age
            .remove(&(effect.pool, effect.applied_at, effect.id));
        Some(effect)
    }
    /// Stronger-stack rejection changes nothing. Equal/higher replaces the whole container
    /// with a fresh identity. Otherwise evict oldest `(applied_at, EffectId)` in its pool.
    pub fn admit(
        &mut self,
        effect: ActiveEffect,
        capacities: EffectCapacities,
    ) -> Result<EffectAdmission, EffectError> {
        effect.validate()?;
        if self.entries.contains_key(&effect.id) {
            return Err(EffectError::Invalid("effect ID reused"));
        }
        let conflict = self.stacks.get(&effect.stack_key()).copied();
        if let Some(existing) = conflict.and_then(|id| self.entries.get(&id)) {
            if existing.abnormal_level > effect.abnormal_level {
                return Ok(EffectAdmission::StrongerStack {
                    existing: existing.id,
                });
            }
        }
        let mut removed = Vec::new();
        if let Some(old) = conflict.and_then(|id| self.remove(id)) {
            removed.push(old);
        }
        while self.pool_count(effect.pool) >= capacities.limit(effect.pool) {
            let oldest = self
                .age
                .iter()
                .find(|(pool, _, _)| *pool == effect.pool)
                .map(|(_, _, id)| *id)
                .ok_or(EffectError::Invalid("missing pool index"))?;
            if let Some(old) = self.remove(oldest) {
                removed.push(old);
            }
        }
        self.insert(effect);
        Ok(EffectAdmission::Applied { removed })
    }
    /// Slot-cap shrink immediately evicts oldest buff containers in the same stable order.
    pub fn shrink(&mut self, capacities: EffectCapacities) -> Vec<ActiveEffect> {
        let mut removed = Vec::new();
        while self.pool_count(EffectPool::Buff) > capacities.limit(EffectPool::Buff) {
            let oldest = self
                .age
                .iter()
                .find(|(pool, _, _)| *pool == EffectPool::Buff)
                .map(|(_, _, id)| *id);
            let Some(old) = oldest.and_then(|id| self.remove(id)) else {
                break;
            };
            removed.push(old);
        }
        removed
    }
    /// Local expiry ordering; the zone merges target ordinal/id between deadline and effect.
    pub fn expired(&self, now: Tick) -> Vec<(Tick, EffectId)> {
        let mut due: Vec<_> = self
            .entries
            .values()
            .filter_map(|effect| {
                effect
                    .expires_at
                    .filter(|until| *until <= now)
                    .map(|until| (until, effect.id))
            })
            .collect();
        due.sort_unstable();
        due
    }
    /// Due periodic callbacks in stable local order. Expiry precludes an equal-tick pulse.
    pub fn pulses(&self, now: Tick) -> Result<Vec<(Tick, EffectId, u32)>, EffectError> {
        let mut due = Vec::new();
        for effect in self
            .entries
            .values()
            .filter(|effect| effect.expires_at.is_none_or(|until| until > now))
        {
            for handler in &effect.handlers {
                if let Some(periodic) = &handler.periodic {
                    let tick = periodic.schedule.due_tick()?;
                    if tick <= now {
                        due.push((tick, effect.id, handler.xml_ordinal));
                    }
                }
            }
        }
        due.sort_unstable();
        Ok(due)
    }
    /// Advance one canonical callback after either resolution or suppression.
    pub fn advance_pulse(&mut self, id: EffectId, ordinal: u32) -> Result<(), EffectError> {
        self.entries
            .get_mut(&id)
            .and_then(|effect| {
                effect
                    .handlers
                    .iter_mut()
                    .find(|handler| handler.xml_ordinal == ordinal)
            })
            .and_then(|handler| handler.periodic.as_mut())
            .ok_or(EffectError::Invalid("missing periodic handler"))?
            .schedule
            .advance()
    }
}
impl<'de> Deserialize<'de> for EffectList {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Snapshot {
            entries: BTreeMap<EffectId, ActiveEffect>,
        }
        let snapshot = Snapshot::deserialize(deserializer)?;
        Self::from_entries(snapshot.entries).map_err(serde::de::Error::custom)
    }
}

/// Supported continuous handlers; instant handlers execute before container admission and
/// never survive as retained handler state. Source AbstractEffect.isInstant overrides.
pub fn continuous_handler(name: &str) -> bool {
    matches!(
        name,
        "Buff"
            | "Debuff"
            | "Passive"
            | "Stun"
            | "Sleep"
            | "Root"
            | "MaxHp"
            | "MaxMp"
            | "TickHp"
            | "TickMp"
            | "DefenceTrait"
            | "AttackTrait"
    )
}

/// Compile only the periodic constructor inputs; this performs no pulse or RNG draw.
/// `TickHp.java:43-45`, `TickMp.java:40-42`, `AbstractEffect.java:148-149`.
pub fn compile_periodic(
    handler: &super::skill::SkillHandler,
    rank: u32,
    max_rank: u32,
) -> Result<Option<(PeriodicResource, PeriodicPower, u32)>, EffectError> {
    let resource = match handler.name.as_str() {
        "TickHp" => PeriodicResource::Hp,
        "TickMp" => PeriodicResource::Mp,
        _ => return Ok(None),
    };
    let parameter = |name: &str| {
        handler
            .params
            .get(name)
            .and_then(|value| value.at(rank, max_rank))
            .map(String::as_str)
    };
    let ticks = parameter("ticks")
        .ok_or(EffectError::Invalid("missing periodic interval"))?
        .parse::<u32>()
        .map_err(|_| EffectError::Invalid("bad periodic interval"))?;
    let (interval, multiplier) =
        super::skill_math::periodic_interval(ticks, 666).map_err(|_| EffectError::Overflow)?;
    let power = ExactRatio::parse_literal(parameter("power").unwrap_or("0"))
        .and_then(ExactRatio::binary64)
        .map_err(|_| EffectError::Invalid("bad periodic power"))?;
    let payload = match parameter("mode").unwrap_or("DIFF") {
        "DIFF" => PeriodicPower::Diff {
            amount: FrozenRatio::new(
                periodic_amount(power, multiplier, 0, false).map_err(|_| EffectError::Overflow)?,
            ),
        },
        "PER" => PeriodicPower::Per {
            power: FrozenRatio::new(power),
            multiplier: FrozenRatio::new(multiplier),
        },
        _ => return Err(EffectError::Invalid("bad periodic mode")),
    };
    Ok(Some((resource, payload, interval)))
}

impl ActiveEffect {
    /// Validate all retained source-derived fields against this epoch's immutable registry.
    pub fn validate_source(
        &self,
        registry: &super::skill::SkillRegistry,
    ) -> Result<(), EffectError> {
        self.validate()?;
        let skill = registry
            .get(self.skill.id)
            .ok_or(EffectError::Invalid("unknown effect skill"))?;
        if skill.runtime_status != crate::domain::class::RuntimeSkillStatus::Executable
            || skill.operate_type == "P"
            || self.skill.rank > skill.max_level
        {
            return Err(EffectError::Invalid("effect skill is deferred/passive or rank invalid"));
        }
        let abnormal = skill
            .abnormal
            .as_ref()
            .ok_or(EffectError::Invalid("effect lacks abnormal descriptor"))?;
        let index = usize::try_from(self.skill.rank.saturating_sub(1))
            .map_err(|_| EffectError::Overflow)?;
        let pool = if abnormal.is_debuff {
            EffectPool::Debuff
        } else if skill.is_magic == 3 {
            EffectPool::SongDance
        } else {
            EffectPool::Buff
        };
        if self.pool != pool
            || self.abnormal_type != abnormal.r#type
            || abnormal.level.get(index) != Some(&self.abnormal_level)
        {
            return Err(EffectError::Invalid("pool/abnormal metadata mismatch"));
        }
        let base_seconds = abnormal
            .time_s
            .get(index)
            .copied()
            .ok_or(EffectError::Invalid("missing abnormal duration"))?;
        if skill.operate_type != "A3" {
            let maximum = u64::try_from(base_seconds)
                .ok()
                .and_then(|seconds| seconds.checked_mul(10))
                .ok_or(EffectError::Invalid("invalid abnormal duration"))?;
            let duration = self
                .expires_at
                .and_then(|expiry| expiry.0.checked_sub(self.applied_at.0))
                .ok_or(EffectError::Invalid("missing finite effect expiry"))?;
            if duration > maximum {
                return Err(EffectError::Invalid("effect duration exceeds source maximum"));
            }
        }
        let flag = |value: &super::skill::RankValue<bool>| {
            value
                .at(self.skill.rank, skill.max_level)
                .copied()
                .ok_or(EffectError::Invalid("missing effect flags"))
        };
        let flags = EffectFlags {
            stay_after_death: flag(&skill.stay_after_death)?,
            stay_on_subclass_change: flag(&skill.stay_on_subclass_change)?,
            removed_on_damage: flag(&skill.removed_on_damage)?,
            removed_on_action_except_move: flag(&skill.removed_on_action_except_move)?,
            irreplaceable: flag(&skill.irreplaceable_buff)?,
        };
        if self.flags != flags {
            return Err(EffectError::Invalid("lifecycle flags mismatch"));
        }
        for handler in &self.handlers {
            let definition = usize::try_from(handler.xml_ordinal)
                .ok()
                .and_then(|index| skill.effects.get(index))
                .ok_or(EffectError::Invalid("unknown handler ordinal"))?;
            if !continuous_handler(&definition.name) {
                return Err(EffectError::Invalid("instant handler retained as continuous"));
            }
            match (
                compile_periodic(definition, self.skill.rank, skill.max_level)?,
                &handler.periodic,
            ) {
                (None, None) => {},
                (Some((resource, power, interval)), Some(periodic))
                    if resource == periodic.resource
                        && power == periodic.power
                        && interval == periodic.schedule.interval_ms => {},
                _ => return Err(EffectError::Invalid("periodic payload differs from source")),
            }
        }
        Ok(())
    }

    /// Freeze the strict >100-tick offline subset after all due callbacks at this tick.
    /// Untimed system modifiers are reconstructed from learned ranks/death-penalty scalar.
    pub fn retain(&self, checkpoint: Tick) -> Result<Option<RetainedEffect>, EffectError> {
        if checkpoint < self.applied_at {
            return Err(EffectError::Invalid("checkpoint before effect application"));
        }
        let Some(remaining_ticks) = self
            .expires_at
            .and_then(|expiry| expiry.0.checked_sub(checkpoint.0))
            .filter(|remaining| *remaining > 100)
        else {
            return Ok(None);
        };
        let handlers = self
            .handlers
            .iter()
            .map(|handler| {
                let periodic = handler
                    .periodic
                    .as_ref()
                    .map(|periodic| {
                        Ok(RetainedPeriodic {
                            resource: periodic.resource,
                            power: periodic.power,
                            interval_ms: periodic.schedule.interval_ms,
                            pulse_index: periodic.schedule.pulse_index,
                            next_in_ms: periodic.schedule.remaining_ms(checkpoint)?,
                        })
                    })
                    .transpose()?;
                Ok(RetainedHandler {
                    xml_ordinal: handler.xml_ordinal,
                    periodic,
                })
            })
            .collect::<Result<Vec<_>, EffectError>>()?;
        Ok(Some(RetainedEffect {
            skill: self.skill,
            source: self.source,
            target_incarnation: self.target_incarnation,
            pool: self.pool,
            abnormal_type: self.abnormal_type.clone(),
            abnormal_level: self.abnormal_level,
            remaining_ticks,
            flags: self.flags,
            handlers,
        }))
    }
}

impl RetainedEffect {
    /// Rebase only paused timers and identities at actual admission. Handler state is
    /// reconstructed declaratively: no onStart/heal/instant/RNG executes in this method.
    pub fn restore(
        &self,
        id: EffectId,
        incarnation: u32,
        admission: Tick,
        registry: &super::skill::SkillRegistry,
    ) -> Result<ActiveEffect, EffectError> {
        if self.remaining_ticks <= 100 || self.skill.id == SkillId(5076) {
            return Err(EffectError::Invalid("retained threshold/system effect"));
        }
        if self
            .handlers
            .iter()
            .filter_map(|handler| handler.periodic.as_ref())
            .any(|periodic| {
                periodic.next_in_ms == 0 || periodic.next_in_ms > u64::from(periodic.interval_ms)
            })
        {
            return Err(EffectError::Invalid("retained periodic phase outside interval"));
        }
        let expires_at = Tick(
            admission
                .0
                .checked_add(self.remaining_ticks)
                .ok_or(EffectError::Overflow)?,
        );
        let handlers = self
            .handlers
            .iter()
            .map(|handler| ActiveHandler {
                xml_ordinal: handler.xml_ordinal,
                periodic: handler.periodic.as_ref().map(|periodic| ActivePeriodic {
                    resource: periodic.resource,
                    power: periodic.power,
                    schedule: PulseSchedule {
                        anchor: admission,
                        next_ideal_offset_ms: periodic.next_in_ms,
                        pulse_index: periodic.pulse_index,
                        interval_ms: periodic.interval_ms,
                    },
                }),
            })
            .collect();
        let effect = ActiveEffect {
            id,
            skill: self.skill,
            source: self.source,
            target_incarnation: incarnation,
            applied_at: admission,
            expires_at: Some(expires_at),
            pool: self.pool,
            abnormal_type: self.abnormal_type.clone(),
            abnormal_level: self.abnormal_level,
            flags: self.flags,
            handlers,
        };
        effect.validate_source(registry)?;
        Ok(effect)
    }
}

/// Derived status provider union. Bits are stable public protocol data; no private death
/// penalty state belongs in this mask. Unknown serialized bits are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct StatusFlags(u64);
impl StatusFlags {
    /// Stun prevents movement, attack and casting.
    pub const STUN: Self = Self(1);
    /// Sleep prevents movement, attack and casting.
    pub const SLEEP: Self = Self(2);
    /// Root prevents movement/chase but permits in-range actions.
    pub const ROOT: Self = Self(4);
    /// Magic-only mute.
    pub const MAGIC_MUTE: Self = Self(8);
    /// Physical-skill mute.
    pub const PHYSICAL_MUTE: Self = Self(16);
    /// Paralyze prevents movement, attack and casting.
    pub const PARALYZE: Self = Self(32);
    /// Source-confirmed resurrection block.
    pub const RESURRECTION_BLOCK: Self = Self(64);
    /// Source-confirmed debuff block.
    pub const DEBUFF_BLOCK: Self = Self(128);
    /// Stable mask for public projection and canonical derived-state comparison.
    pub const fn bits(self) -> u64 {
        self.0
    }
    /// Add providers without ever clearing another effect's contribution.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    /// Whether any named status is present.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}
impl From<StatusFlags> for u64 {
    fn from(value: StatusFlags) -> Self {
        value.0
    }
}
impl TryFrom<u64> for StatusFlags {
    type Error = EffectError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value & !255 != 0 {
            return Err(EffectError::Invalid("unknown status flag"));
        }
        Ok(Self(value))
    }
}
impl EffectList {
    /// Rebuild from all current providers after removal/replacement, never clear one bit
    /// merely because one effect stopped. Immutable registry resolves handler identities.
    pub fn status_flags(
        &self,
        registry: &super::skill::SkillRegistry,
    ) -> Result<StatusFlags, EffectError> {
        let mut flags = StatusFlags::default();
        for effect in self.entries.values() {
            effect.validate_source(registry)?;
            let definition = registry
                .get(effect.skill.id)
                .ok_or(EffectError::Invalid("unknown status source"))?;
            for handler in &effect.handlers {
                let source = usize::try_from(handler.xml_ordinal)
                    .ok()
                    .and_then(|i| definition.effects.get(i))
                    .ok_or(EffectError::Invalid("unknown status handler"))?;
                let flag = match source.name.as_str() {
                    "Stun" => StatusFlags::STUN,
                    "Sleep" => StatusFlags::SLEEP,
                    "Root" => StatusFlags::ROOT,
                    _ => StatusFlags::default(),
                };
                flags = flags.union(flag);
            }
        }
        Ok(flags)
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
    use crate::infrastructure::skill_data::{load_skills, SkillSource};

    fn entry(id: u64, skill: u32, applied: u64, pool: EffectPool) -> ActiveEffect {
        ActiveEffect {
            id: EffectId(id),
            skill: SkillRef {
                id: SkillId(skill),
                rank: 1,
            },
            source: EffectSource {
                kind: EffectSourceKind::System,
                binding: None,
            },
            target_incarnation: 1,
            applied_at: Tick(applied),
            expires_at: Some(Tick(applied + 1000)),
            pool,
            abnormal_type: "NONE".into(),
            abnormal_level: 0,
            flags: EffectFlags {
                stay_after_death: false,
                stay_on_subclass_change: true,
                removed_on_damage: false,
                removed_on_action_except_move: false,
                irreplaceable: false,
            },
            handlers: vec![],
        }
    }
    fn source_entry(
        registry: &super::super::skill::SkillRegistry,
        id: u64,
        skill_id: u32,
    ) -> ActiveEffect {
        let skill = registry.get(SkillId(skill_id)).unwrap();
        let abnormal = skill.abnormal.as_ref().unwrap();
        let mut effect = entry(
            id,
            skill_id,
            0,
            if abnormal.is_debuff {
                EffectPool::Debuff
            } else {
                EffectPool::Buff
            },
        );
        effect.expires_at = Some(Tick(u64::try_from(abnormal.time_s[0]).unwrap() * 10));
        effect.abnormal_type = abnormal.r#type.clone();
        effect.abnormal_level = abnormal.level[0];
        effect.flags = EffectFlags {
            stay_after_death: *skill.stay_after_death.at(1, skill.max_level).unwrap(),
            stay_on_subclass_change: *skill
                .stay_on_subclass_change
                .at(1, skill.max_level)
                .unwrap(),
            removed_on_damage: *skill.removed_on_damage.at(1, skill.max_level).unwrap(),
            removed_on_action_except_move: *skill
                .removed_on_action_except_move
                .at(1, skill.max_level)
                .unwrap(),
            irreplaceable: *skill.irreplaceable_buff.at(1, skill.max_level).unwrap(),
        };
        effect.handlers = skill
            .effects
            .iter()
            .enumerate()
            .filter(|(_, handler)| continuous_handler(&handler.name))
            .map(|(ordinal, handler)| ActiveHandler {
                xml_ordinal: u32::try_from(ordinal).unwrap(),
                periodic: compile_periodic(handler, 1, skill.max_level).unwrap().map(
                    |(resource, power, interval)| ActivePeriodic {
                        resource,
                        power,
                        schedule: PulseSchedule::new(Tick(0), interval).unwrap(),
                    },
                ),
            })
            .collect();
        effect.validate_source(registry).unwrap();
        effect
    }

    #[test]
    fn weaker_stack_is_a_noop_equal_replaces_whole_container() {
        let caps = EffectCapacities::for_enlarge(0);
        let mut list = EffectList::default();
        let mut first = entry(1, 10, 100, EffectPool::Buff);
        first.abnormal_type = "P_ATK_UP".into();
        first.abnormal_level = 3;
        list.admit(first.clone(), caps).unwrap();
        let before = serde_json::to_vec(&list).unwrap();
        let mut lesser = first.clone();
        lesser.id = EffectId(2);
        lesser.skill.id = SkillId(11);
        lesser.abnormal_level = 2;
        lesser.applied_at = Tick(101);
        assert_eq!(
            list.admit(lesser.clone(), caps).unwrap(),
            EffectAdmission::StrongerStack {
                existing: EffectId(1)
            }
        );
        assert_eq!(serde_json::to_vec(&list).unwrap(), before);
        lesser.abnormal_level = 3;
        assert_eq!(
            list.admit(lesser, caps).unwrap(),
            EffectAdmission::Applied {
                removed: vec![first]
            }
        );
        assert_eq!(list.entries().len(), 1);
        assert!(list.entries().contains_key(&EffectId(2)));
    }

    #[test]
    fn pools_are_independent_and_eviction_uses_application_time_then_id() {
        let caps = EffectCapacities::for_enlarge(0);
        let mut list = EffectList::default();
        for (pool, count, offset) in [
            (EffectPool::Buff, 20, 0),
            (EffectPool::SongDance, 12, 100),
            (EffectPool::Debuff, 8, 200),
        ] {
            for i in 1..=count {
                list.admit(
                    entry(offset + i, u32::try_from(offset + i).unwrap(), 100 - i, pool),
                    caps,
                )
                .unwrap();
            }
            let result = list
                .admit(entry(offset + 90, u32::try_from(offset + 90).unwrap(), 100, pool), caps)
                .unwrap();
            let EffectAdmission::Applied { removed } = result else {
                unreachable!();
            };
            assert_eq!(
                removed.iter().map(|effect| effect.id).collect::<Vec<_>>(),
                vec![EffectId(offset + count)]
            );
        }
        assert_eq!(list.entries().len(), 40);
        list.validate_capacity(caps).unwrap();
        let encoded = serde_json::to_vec(&list).unwrap();
        let restored: EffectList = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored, list);
        assert!(!String::from_utf8(encoded).unwrap().contains("stacks"));
    }

    #[test]
    fn capacity_shrink_evicts_oldest_and_none_stacks_only_same_skill() {
        let mut list = EffectList::default();
        for i in 1..=24 {
            list.admit(
                entry(i, u32::try_from(i).unwrap(), 1, EffectPool::Buff),
                EffectCapacities::for_enlarge(999),
            )
            .unwrap();
        }
        let removed = list.shrink(EffectCapacities::for_enlarge(-3));
        assert_eq!(
            removed.iter().map(|effect| effect.id).collect::<Vec<_>>(),
            (1..=4).map(EffectId).collect::<Vec<_>>()
        );
        let EffectAdmission::Applied { removed } = list
            .admit(entry(30, 5, 2, EffectPool::Buff), EffectCapacities::for_enlarge(0))
            .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id, EffectId(5));
        assert_eq!(list.entries().len(), 20);
    }

    #[test]
    fn expiry_precedes_equal_tick_pulse_and_phase_never_drifts() {
        let mut list = EffectList::default();
        let mut effect = entry(1, 1, 0, EffectPool::Debuff);
        effect.expires_at = Some(Tick(14));
        effect.handlers.push(ActiveHandler {
            xml_ordinal: 0,
            periodic: Some(ActivePeriodic {
                resource: PeriodicResource::Hp,
                power: PeriodicPower::Diff {
                    amount: FrozenRatio::new(ExactRatio::whole(-1)),
                },
                schedule: PulseSchedule::new(Tick(0), 666).unwrap(),
            }),
        });
        list.admit(effect, EffectCapacities::for_enlarge(0))
            .unwrap();
        assert!(list.pulses(Tick(6)).unwrap().is_empty());
        assert_eq!(list.pulses(Tick(7)).unwrap(), vec![(Tick(7), EffectId(1), 0)]);
        list.advance_pulse(EffectId(1), 0).unwrap();
        assert_eq!(list.expired(Tick(14)), vec![(Tick(14), EffectId(1))]);
        assert!(list.pulses(Tick(14)).unwrap().is_empty());
        let mut schedule = PulseSchedule::new(Tick(100), 666).unwrap();
        for tick in [107, 114, 120, 127] {
            assert_eq!(schedule.due_tick().unwrap(), Tick(tick));
            schedule.advance().unwrap();
        }
    }

    #[test]
    fn restore_rebuilds_indexes_and_rejects_duplicate_stacks_or_ids() {
        let first = entry(1, 10, 0, EffectPool::Buff);
        let second = entry(2, 10, 1, EffectPool::Buff);
        assert!(EffectList::from_entries(BTreeMap::from([
            (EffectId(1), first.clone()),
            (EffectId(2), second)
        ]))
        .is_err());
        assert!(EffectList::from_entries(BTreeMap::from([(EffectId(2), first.clone())])).is_err());
        let mut list = EffectList::from_entries(BTreeMap::from([(EffectId(1), first)])).unwrap();
        let canonical = serde_json::to_vec(&list).unwrap();
        list.age.clear();
        list.stacks.clear();
        assert_eq!(serde_json::to_vec(&list).unwrap(), canonical);
        let restored: EffectList = serde_json::from_slice(&canonical).unwrap();
        assert_eq!(restored.age.len(), 1);
        assert_eq!(restored.stacks.len(), 1);
    }

    #[test]
    fn retained_periodic_phase_rebases_without_replaying_start_or_resetting_index() {
        let registry = load_skills(&SkillSource::embedded(), &[]).unwrap();
        let mut effect = source_entry(&registry, 7, 1183);
        effect.expires_at = Some(Tick(110));
        effect.handlers[0]
            .periodic
            .as_mut()
            .unwrap()
            .schedule
            .advance()
            .unwrap();
        assert!(effect.retain(Tick(10)).unwrap().is_none());
        let retained = effect.retain(Tick(9)).unwrap().unwrap();
        assert_eq!(retained.remaining_ticks, 101);
        let pulse = retained.handlers[0].periodic.as_ref().unwrap();
        assert_eq!((pulse.pulse_index, pulse.next_in_ms), (2, 432));
        let bytes = serde_json::to_vec(&retained).unwrap();
        let retained: RetainedEffect = serde_json::from_slice(&bytes).unwrap();
        let restored = retained
            .restore(EffectId(100), 4, Tick(1000), &registry)
            .unwrap();
        assert_eq!(
            (restored.id, restored.target_incarnation, restored.expires_at),
            (EffectId(100), 4, Some(Tick(1101)))
        );
        assert_eq!(
            restored.handlers[0]
                .periodic
                .as_ref()
                .unwrap()
                .schedule
                .due_tick()
                .unwrap(),
            Tick(1005)
        );
        assert_eq!(
            restored.handlers[0].periodic.as_ref().unwrap().power,
            effect.handlers[0].periodic.as_ref().unwrap().power
        );
        let mut invalid = retained.clone();
        invalid.remaining_ticks = 100;
        assert!(invalid
            .restore(EffectId(101), 4, Tick(1000), &registry)
            .is_err());
        invalid = retained;
        invalid.handlers[0].periodic.as_mut().unwrap().next_in_ms = 667;
        assert!(invalid
            .restore(EffectId(101), 4, Tick(1000), &registry)
            .is_err());
    }

    #[test]
    fn multiple_status_providers_survive_removal_and_source_payload_tampering_fails() {
        let registry = load_skills(&SkillSource::embedded(), &[]).unwrap();
        let mut list = EffectList::default();
        // Ultimate Defense and Dryad Root have different abnormal keys but both root.
        list.admit(source_entry(&registry, 1, 110), EffectCapacities::for_enlarge(0))
            .unwrap();
        list.admit(source_entry(&registry, 2, 1201), EffectCapacities::for_enlarge(0))
            .unwrap();
        assert_eq!(list.entries().len(), 2);
        assert!(list
            .status_flags(&registry)
            .unwrap()
            .intersects(StatusFlags::ROOT));
        list.remove(EffectId(1));
        assert!(list
            .status_flags(&registry)
            .unwrap()
            .intersects(StatusFlags::ROOT));
        list.remove(EffectId(2));
        assert_eq!(list.status_flags(&registry).unwrap().bits(), 0);
        let mut poisoned = source_entry(&registry, 3, 1183);
        poisoned.handlers[0].periodic.as_mut().unwrap().power = PeriodicPower::Diff {
            amount: FrozenRatio::new(ExactRatio::whole(-999)),
        };
        assert!(poisoned.validate_source(&registry).is_err());
        assert!(serde_json::from_str::<StatusFlags>("256").is_err());
        assert!(serde_json::from_str::<FrozenRatio>("\"2/4\"").is_err());
    }
}

/// Stable source lifecycle removal causes; replacement allocates a different `EffectId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum EffectRemoveReason {
    Expired,
    Replaced,
    Overflow,
    Dispelled,
    Damage,
    Death,
    Transfer,
}
