//! V5 command-phase staging and canonical cast lifecycle. This module never emits transport
//! responses: the tick driver settles staged dispositions at the new-intent phase.
//! Pinned L2Character.java:1495-1543,1664-1675,4617-4642 and CharStat.java:482-514.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::effects::{FrozenRatio, StatusFlags, TargetBinding};
use super::skill::{SkillDef, SkillId, SkillRef, SkillRegistry};
use super::skill_math::{affect_limit, cast_animation, cast_deadlines, ExactRatio};
use super::stat_functions::{FunctionConditions, StatCalculator};
use super::{
    CastId, CastInFlight, CastState, EntityId, EntityOrdinal, Ordinal, Phase3Entity, StatError,
    Tick, Q,
};
use crate::domain::class::RuntimeSkillStatus;
use crate::domain::subclass::LearnedSkill;

/// Cancellation values correspond exactly to the accepted additive wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum CastCancelReason {
    Stopped,
    Moved,
    Damage,
    Incapacitated,
    TargetLost,
    OutOfRange,
    Resources,
    CasterDied,
    PeaceZone,
    Transfer,
}

/// Start refusals are dispositions; failures after an accepted start are cancellations.
/// Arithmetic failures instead fence the draft and may never become a guessed result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(missing_docs)]
pub enum CastError {
    #[error("unknown skill")]
    SkillUnknown,
    #[error("skill is not learned at this rank")]
    SkillNotLearned,
    #[error("skill capability is deferred")]
    NotYetImplemented,
    #[error("passive skill cannot be cast")]
    InvalidSkill,
    #[error("caster is dead")]
    DeadActor,
    #[error("skill is on reuse")]
    SkillCooldown,
    #[error("insufficient MP")]
    InsufficientMp,
    #[error("insufficient HP")]
    InsufficientHp,
    #[error("already casting")]
    AlreadyCasting,
    #[error("action disabled")]
    Incapacitated,
    #[error("matching skill kind is muted")]
    Silenced,
    #[error("skill condition failed")]
    Condition,
    #[error("invalid cast state or arithmetic overflow")]
    InvalidState,
}
impl From<StatError> for CastError {
    fn from(_: StatError) -> Self {
        Self::InvalidState
    }
}

/// Read the actor's authoritative learned ledger, never the requested next rank or client
/// metadata. This performs no timing/resource checks that could change before phase8.
pub fn learned_skill<'a>(
    registry: &'a SkillRegistry,
    learned: &[LearnedSkill],
    id: SkillId,
) -> Result<(&'a SkillDef, SkillRef), CastError> {
    let skill = registry.get(id).ok_or(CastError::SkillUnknown)?;
    let rank = learned
        .iter()
        .find(|entry| entry.key == skill.key)
        .map(|entry| entry.level)
        .ok_or(CastError::SkillNotLearned)?;
    if rank == 0 || rank > skill.max_level {
        return Err(CastError::SkillNotLearned);
    }
    if skill.runtime_status != RuntimeSkillStatus::Executable {
        return Err(CastError::NotYetImplemented);
    }
    if skill.operate_type == "P" {
        return Err(CastError::InvalidSkill);
    }
    Ok((skill, SkillRef { id, rank }))
}

/// A command-phase binding may name an unknown target. Preserve that identity for a later
/// refusal rather than silently switching to the actor's newer selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct CapturedTarget {
    pub entity: EntityId,
    pub incarnation: Option<u32>,
}

/// Phase-local command entry. A canceled staged start remains in receive order so its
/// command is acknowledged without creating a cast, paying costs or consuming RNG.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct StagedCast {
    pub ordinal: Ordinal,
    pub caster: TargetBinding,
    pub skill_id: SkillId,
    pub target: Option<CapturedTarget>,
    pub canceled: Option<CastCancelReason>,
}

/// Receive-ordered actions with command-time identity bindings and phase-eight guards.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum StagedAction {
    Cast(StagedCast),
    Attack {
        ordinal: Ordinal,
        caster: TargetBinding,
        target: Option<CapturedTarget>,
        canceled: bool,
    },
    AcceptResurrect {
        ordinal: Ordinal,
        target: TargetBinding,
        offer_id: super::OfferId,
        accept: bool,
        present: bool,
    },
}
impl StagedAction {
    /// Original receive ordinal shared by all three action types.
    pub const fn ordinal(&self) -> Ordinal {
        match self {
            Self::Cast(cast) => cast.ordinal,
            Self::Attack { ordinal, .. } | Self::AcceptResurrect { ordinal, .. } => *ordinal,
        }
    }
}

/// Scratch queue only; never snapshot/digest this queue or replay it on gate retry.
#[derive(Debug, Default)]
pub struct StagedActions {
    entries: Vec<StagedAction>,
    /// Accepted and recorded Known receipts share the same tick-local deduplication scope.
    pub(crate) item_receipts: BTreeMap<
        (crate::domain::account::AccountId, super::phase4_projection::ItemReceiptRef),
        ItemReceiptState,
    >,
}
/// Tick-local receipt knowledge; a recorded conflict must not be bypassed by a later fresh intent.
#[derive(Debug)]
pub(crate) enum ItemReceiptState {
    Known(super::phase4_projection::ItemReceiptFact),
    Conflict(String),
}
impl StagedActions {
    /// Called only after source/generation authorization. Target is captured at this
    /// command's ordinal; rank/resources/expiry-dependent eligibility are checked later.
    pub fn stage(&mut self, entry: StagedCast) -> Result<(), CastError> {
        if entry.caster.incarnation == 0 || entry.canceled.is_some() {
            return Err(CastError::InvalidState);
        }
        self.stage_action(StagedAction::Cast(entry))
    }
    /// Capture an attack/offer without evaluating expiry-dependent gameplay guards.
    pub fn stage_action(&mut self, entry: StagedAction) -> Result<(), CastError> {
        if self
            .entries
            .last()
            .is_some_and(|previous| previous.ordinal() >= entry.ordinal())
        {
            return Err(CastError::InvalidState);
        }
        self.entries.push(entry);
        Ok(())
    }
    /// A specific live cast id cannot name an unallocated staged cast. An unguarded cancel
    /// cancels all earlier staged starts, never a later command.
    pub fn cancel(&mut self, caster: EntityId, cast_id: Option<CastId>, reason: CastCancelReason) {
        if cast_id.is_none() {
            for entry in &mut self.entries {
                if let StagedAction::Cast(entry) = entry {
                    if entry.caster.entity == caster && entry.canceled.is_none() {
                        entry.canceled = Some(reason);
                    }
                }
            }
        }
    }
    /// Uncanceled prior intents in receive order, for command-phase engagement checks.
    pub fn pending(&self) -> impl Iterator<Item = &StagedCast> {
        self.entries.iter().filter_map(|entry| match entry {
            StagedAction::Cast(cast) if cast.canceled.is_none() => Some(cast),
            StagedAction::Cast(_)
            | StagedAction::Attack { .. }
            | StagedAction::AcceptResurrect { .. } => None,
        })
    }
    /// A later valid stop/move/selection command suppresses prior attack starts.
    pub fn cancel_attacks(&mut self, entity: EntityId) {
        for entry in &mut self.entries {
            if let StagedAction::Attack {
                caster, canceled, ..
            } = entry
            {
                if caster.entity == entity {
                    *canceled = true;
                }
            }
        }
    }
    /// Pending attacks participate in command-phase learning engagement checks.
    pub fn pending_attacks(
        &self,
    ) -> impl Iterator<Item = (TargetBinding, Option<CapturedTarget>)> + '_ {
        self.entries.iter().filter_map(|entry| match entry {
            StagedAction::Attack {
                caster,
                target,
                canceled: false,
                ..
            } => Some((*caster, *target)),
            StagedAction::Cast(_)
            | StagedAction::Attack { .. }
            | StagedAction::AcceptResurrect { .. } => None,
        })
    }
    /// Consume once at phase8, retaining cross-actor receive order and canceled entries.
    pub fn drain(&mut self) -> impl Iterator<Item = StagedAction> + '_ {
        self.entries.drain(..)
    }
}

/// Current effective input for timing/cost getters. No rounded legacy sheet is reconstructed
/// here; the live effective cache supplies speeds and the ordered calculator.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct CastStatContext<'a> {
    pub player_level: Option<u32>,
    pub calculator: &'a StatCalculator,
    pub conditions: FunctionConditions<'a>,
    pub attack_speed: u32,
    pub cast_speed: u32,
    pub flags: StatusFlags,
}

/// Source getters before start validation. Completion recomputes MP2 from the current
/// calculator (L2Character.java:4617); frozen `mp_end` is the start precheck/projection only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct CastTerms {
    pub skill: SkillRef,
    pub animation: FrozenRatio,
    pub continuous: bool,
    pub mp_start: u32,
    pub mp_end: u32,
    pub hp_end: u32,
    pub reuse_ms: u32,
}

fn rank_cost(values: &[i64], rank: u32) -> Result<i64, CastError> {
    rank.checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|i| values.get(i))
        .copied()
        .filter(|v| *v >= 0)
        .ok_or(CastError::InvalidState)
}
fn cost_q(value: i64) -> Result<i64, CastError> {
    value.checked_mul(Q).ok_or(CastError::InvalidState)
}
fn whole_cost(value: i64) -> Result<u32, CastError> {
    if value < 0 {
        return Err(CastError::InvalidState);
    }
    u32::try_from(value.checked_div(Q).ok_or(CastError::InvalidState)?)
        .map_err(|_| CastError::InvalidState)
}

/// Distinct MP1 and MP2 getter folds; neither truncates before all applicable Funcs.
/// Dance additional-MP requires its configured source input and is intentionally rejected
/// here until an executable dance exists in the validated catalogue, never approximated.
pub fn cast_costs(
    skill: &SkillDef,
    rank: u32,
    context: CastStatContext<'_>,
) -> Result<(u32, u32, u32), CastError> {
    if rank == 0 || rank > skill.max_level || skill.is_magic == 3 || skill.is_magic > 4 {
        return Err(CastError::InvalidState);
    }
    let fold = |stat, initial| {
        context
            .calculator
            .fold(stat, initial, context.conditions)
            .map_err(CastError::from)
    };
    let mp1 = whole_cost(fold("mpConsume", cost_q(rank_cost(&skill.mp_consume_start, rank)?)?)?)?;
    let mp2_q = fold("mpConsume", cost_q(rank_cost(&skill.mp_consume_end, rank)?)?)?;
    let mp2 = whole_cost(fold(
        if skill.is_magic == 1 {
            "magicalMpConsumeRate"
        } else {
            "physicalMpConsumeRate"
        },
        mp2_q,
    )?)?;
    let hp =
        u32::try_from(rank_cost(&skill.hp_consume, rank)?).map_err(|_| CastError::InvalidState)?;
    Ok((mp1, mp2, hp))
}

/// Prepare source-derived timing/costs at accepted-start time, after phase2/3 changes.
/// Skills with unsupported capabilities and passives are never partially executable.
pub fn cast_terms(
    skill: &SkillDef,
    rank: u32,
    context: CastStatContext<'_>,
) -> Result<CastTerms, CastError> {
    if skill.runtime_status != RuntimeSkillStatus::Executable {
        return Err(CastError::NotYetImplemented);
    }
    if !matches!(skill.operate_type.as_str(), "A1" | "A2" | "A3") {
        return Err(CastError::InvalidSkill);
    }
    if context.flags.intersects(
        StatusFlags::STUN
            .union(StatusFlags::SLEEP)
            .union(StatusFlags::PARALYZE),
    ) {
        return Err(CastError::Incapacitated);
    }
    if skill.is_magic != 2
        && context.flags.intersects(if skill.is_magic == 1 {
            StatusFlags::MAGIC_MUTE
        } else {
            StatusFlags::PHYSICAL_MUTE
        })
    {
        return Err(CastError::Silenced);
    }
    for condition in &skill.conditions {
        if !context
            .conditions
            .matches_at_rank(condition, context.player_level, rank)?
        {
            return Err(CastError::Condition);
        }
    }
    let read = |values: &super::skill::RankValue<i64>| -> Result<u32, CastError> {
        values
            .at(rank, skill.max_level)
            .copied()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(CastError::InvalidState)
    };
    let (mp_start, mp_end, hp_end) = cast_costs(skill, rank, context)?;
    let animation = cast_animation(
        read(&skill.hit_time_ms)?,
        if skill.is_magic == 1 {
            context.cast_speed
        } else {
            context.attack_speed
        },
        skill.is_magic == 2,
        skill.is_magic == 1,
        false,
    )?;
    let reuse_base = read(&skill.reuse_ms)?;
    let locked = *skill
        .reuse_delay_lock
        .at(rank, skill.max_level)
        .ok_or(CastError::InvalidState)?;
    let reuse_ms = if locked || skill.is_magic == 2 {
        reuse_base
    } else {
        let multiplier = context.calculator.fold(
            match skill.is_magic {
                1 => "mReuse",
                0 => "pReuse",
                _ => "dReuse",
            },
            Q,
            context.conditions,
        )?;
        let value = ExactRatio::whole(i64::from(reuse_base))
            .checked_mul(ExactRatio::new(i128::from(multiplier), i128::from(Q))?)?
            .trunc()?;
        u32::try_from(value).map_err(|_| CastError::InvalidState)?
    };
    Ok(CastTerms {
        skill: SkillRef { id: skill.id, rank },
        animation: FrozenRatio::new(animation),
        continuous: skill.operate_type != "A1",
        mp_start,
        mp_end,
        hp_end,
        reuse_ms,
    })
}

/// Candidate eligibility/range/peace is computed by the zone before this ordering step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct AffectCandidate {
    pub ordinal: EntityOrdinal,
    pub binding: TargetBinding,
}

/// Choose once at accepted start. Launch/impact may only filter these bindings; no resample.
/// Zero width is unlimited even with nonzero offset (C1); positive width consumes one roll.
pub fn collect_affect_targets(
    mut candidates: Vec<AffectCandidate>,
    offset: u32,
    width: u32,
    roll: Option<u32>,
) -> Result<(u32, Vec<TargetBinding>), CastError> {
    let limit = affect_limit(offset, width, roll)?;
    candidates.sort_by_key(|c| (c.ordinal, c.binding.entity));
    let mut seen = BTreeSet::new();
    if candidates
        .iter()
        .any(|c| c.ordinal.0 == 0 || c.binding.incarnation == 0 || !seen.insert(c.binding.entity))
    {
        return Err(CastError::InvalidState);
    }
    if limit != 0 {
        candidates.truncate(usize::try_from(limit).map_err(|_| CastError::InvalidState)?);
    }
    Ok((limit, candidates.into_iter().map(|c| c.binding).collect()))
}

/// Authoritative start data after target/hostility validation; no client fields reach it.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct CastStart {
    pub now: Tick,
    pub ordinal: Ordinal,
    pub incarnation: u32,
    pub terms: CastTerms,
    pub target: Option<TargetBinding>,
    pub affect_limit: u32,
    pub targets: Vec<TargetBinding>,
}

impl Phase3Entity {
    /// Install canonical cast, cost and reuse only after every checked allocation/deadline.
    /// Caller passes `CombatState`'s MP by reference; no duplicate resource ledger is kept.
    /// A zero-delay launch is performed by the caller immediately, never a recursive impact.
    pub fn begin_cast(
        &mut self,
        next_cast_id: &mut u64,
        mp: &mut u32,
        hp: u32,
        start: CastStart,
    ) -> Result<CastInFlight, CastError> {
        if hp == 0 {
            return Err(CastError::DeadActor);
        }
        if self.cast != CastState::Idle {
            return Err(CastError::AlreadyCasting);
        }
        if self
            .reuse
            .get(&start.terms.skill.id)
            .is_some_and(|until| *until > start.now)
        {
            return Err(CastError::SkillCooldown);
        }
        if u64::from(*mp)
            < u64::from(start.terms.mp_start).saturating_add(u64::from(start.terms.mp_end))
        {
            return Err(CastError::InsufficientMp);
        }
        if hp <= start.terms.hp_end {
            return Err(CastError::InsufficientHp);
        }
        let next_id = next_cast_id.checked_add(1).ok_or(CastError::InvalidState)?;
        if *next_cast_id == 0
            || start.incarnation == 0
            || start.terms.skill.id.0 == 0
            || start.terms.skill.rank == 0
        {
            return Err(CastError::InvalidState);
        }
        let mut seen = BTreeSet::new();
        if start.target.is_some_and(|t| t.incarnation == 0)
            || start
                .targets
                .iter()
                .any(|t| t.incarnation == 0 || !seen.insert(t.entity))
            || (start.affect_limit != 0
                && u64::try_from(start.targets.len()).map_err(|_| CastError::InvalidState)?
                    > u64::from(start.affect_limit))
        {
            return Err(CastError::InvalidState);
        }
        let deadlines =
            cast_deadlines(start.now, start.terms.animation.value(), start.terms.continuous)?;
        let reuse_ticks = if start.terms.reuse_ms > 10 {
            u64::from(start.terms.reuse_ms).div_ceil(100)
        } else {
            0
        };
        let reuse_until = Tick(
            start
                .now
                .0
                .checked_add(reuse_ticks)
                .ok_or(CastError::InvalidState)?,
        );
        let remaining_mp = mp
            .checked_sub(start.terms.mp_start)
            .ok_or(CastError::InvalidState)?;
        let cast = CastInFlight {
            id: CastId(*next_cast_id),
            caster_incarnation: start.incarnation,
            skill: start.terms.skill,
            command_ordinal: start.ordinal,
            start_at: start.now,
            animation: start.terms.animation,
            launch_at: deadlines.launch_at,
            complete_at: deadlines.complete_at,
            interrupt_until: deadlines.interrupt_until,
            launched: false,
            target: start.target,
            affect_limit: start.affect_limit,
            targets: start.targets,
            mp_start: start.terms.mp_start,
            mp_end: start.terms.mp_end,
            hp_end: start.terms.hp_end,
            reuse_until,
        };
        *mp = remaining_mp;
        *next_cast_id = next_id;
        if reuse_ticks != 0 {
            self.reuse.insert(cast.skill.id, reuse_until);
        }
        self.cast = CastState::Casting(cast.clone());
        Ok(cast)
    }

    /// Mark a due launch and retain only still-valid bound lives in their original order.
    /// Repeated/stale work is a no-op and consumes no callbacks/RNG.
    pub fn launch_cast(
        &mut self,
        now: Tick,
        id: CastId,
        mut valid: impl FnMut(TargetBinding) -> bool,
    ) -> bool {
        let CastState::Casting(cast) = &mut self.cast else {
            return false;
        };
        cast.id == id && cast.launch(now, &mut valid)
    }

    /// Cancel exactly this live cast (or any live cast when unguarded). Costs/reuse persist.
    /// The returned canonical state supplies the event's identity and original attribution.
    pub fn cancel_cast(&mut self, id: Option<CastId>) -> Option<CastInFlight> {
        if !matches!(&self.cast, CastState::Casting(cast) if id.is_none_or(|id| cast.id == id)) {
            return None;
        }
        match std::mem::replace(&mut self.cast, CastState::Idle) {
            CastState::Casting(cast) => Some(cast),
            CastState::Idle => None,
        }
    }

    /// Damage callers consume their calcAtkBreak roll BEFORE invoking this cutoff check.
    /// An out-of-window or physical cast cannot cancel, but must not suppress that draw.
    pub fn interrupt_cast(
        &mut self,
        now: Tick,
        success: bool,
        skill: &SkillDef,
    ) -> Option<CastInFlight> {
        let CastState::Casting(cast) = &self.cast else {
            return None;
        };
        if !success
            || cast.skill.id != skill.id
            || !matches!(skill.is_magic, 1 | 2)
            || now < cast.start_at
            || now >= cast.interrupt_until
        {
            return None;
        }
        self.cancel_cast(Some(cast.id))
    }

    /// End the still-current cast only after handlers have finished. A handler may already
    /// have canceled it through damage/death; never clear a different cast identity.
    pub fn finish_cast(&mut self, id: CastId) -> Option<CastInFlight> {
        self.cancel_cast(Some(id))
    }

    /// Current MP2 then HP consumption, exactly in pinned source order. Insufficient HP
    /// after paying MP2 does NOT refund that MP. Failure clears the accepted cast and must
    /// emit Resources cancellation, never a late intent rejection. The caller revalidates
    /// life/target/peace/status first and obtains current costs through `cast_costs`.
    /// Invoke once for each due impact key, then resolve handlers and call `finish_cast`;
    /// this operation intentionally leaves Casting visible during those handlers.
    #[allow(clippy::too_many_arguments)] // Direct resource references avoid a duplicate ledger.
    pub fn pay_cast_impact(
        &mut self,
        now: Tick,
        id: CastId,
        mp: &mut u32,
        hp: &mut u32,
        mp_end: u32,
        hp_end: u32,
    ) -> Result<Option<CastInFlight>, CastCancelReason> {
        let CastState::Casting(cast) = &self.cast else {
            return Ok(None);
        };
        if cast.id != id {
            return Ok(None);
        }
        let result = cast.pay_impact(now, mp, hp, mp_end, hp_end);
        if result.is_err() {
            self.cancel_cast(Some(id));
        }
        result
    }
}

impl CastInFlight {
    /// Shared main/item-lane launch; the canonical target list can only shrink.
    pub fn launch(&mut self, now: Tick, mut valid: impl FnMut(TargetBinding) -> bool) -> bool {
        if self.launched || self.launch_at > now {
            return false;
        }
        self.targets.retain(|target| valid(*target));
        self.launched = true;
        true
    }

    /// Source MP2-before-HP payment shared by both lanes. The owner removes this exact
    /// cast on error and keeps it visible through successful handlers, then finalizes it.
    pub fn pay_impact(
        &self,
        now: Tick,
        mp: &mut u32,
        hp: &mut u32,
        mp_end: u32,
        hp_end: u32,
    ) -> Result<Option<Self>, CastCancelReason> {
        if !self.launched || self.complete_at > now {
            return Ok(None);
        }
        if *hp == 0 {
            return Err(CastCancelReason::CasterDied);
        }
        let Some(remaining_mp) = mp.checked_sub(mp_end) else {
            return Err(CastCancelReason::Resources);
        };
        *mp = remaining_mp;
        if *hp <= hp_end {
            return Err(CastCancelReason::Resources);
        }
        *hp = hp.saturating_sub(hp_end);
        Ok(Some(self.clone()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::super::phase3_entity::tests::blank;
    use super::super::stat_functions::{FunctionKey, FunctionOp, StatFunction};
    use super::super::state_effects::Phase3Schedule;
    use super::*;
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn binding(n: u128) -> TargetBinding {
        TargetBinding {
            entity: EntityId::from_uuid(Uuid::from_u128(n)),
            incarnation: 1,
        }
    }

    #[test]
    fn v6_cast_level_range_uses_resolved_rank_and_inclusive_bounds() {
        use super::super::skill::{PlayerLevelRange, SkillCondition};
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let mut skill = rules.skills().get(SkillId(3)).unwrap().clone();
        let mut ranges = vec![
            PlayerLevelRange {
                minimum: 10,
                maximum: 20
            };
            usize::try_from(skill.max_level).unwrap()
        ];
        ranges[1] = PlayerLevelRange {
            minimum: 30,
            maximum: 40,
        };
        let negate = |condition| SkillCondition::Not {
            conditions: vec![condition],
            attrs: BTreeMap::new(),
        };
        skill.conditions = vec![SkillCondition::And {
            conditions: vec![
                SkillCondition::UsingKind {
                    weapon_kinds: vec!["SWORD".into()],
                },
                negate(negate(SkillCondition::PlayerLevelRange { ranges })),
            ],
            attrs: BTreeMap::new(),
        }];
        let calculator = StatCalculator::default();
        for (rank, level, allowed) in [
            (1, 9, false),
            (1, 10, true),
            (1, 20, true),
            (1, 21, false),
            (2, 29, false),
            (2, 30, true),
            (2, 40, true),
            (2, 41, false),
        ] {
            let mut inputs = context(&calculator);
            inputs.player_level = Some(level);
            let outcome = cast_terms(&skill, rank, inputs);
            if allowed {
                assert!(outcome.is_ok(), "{rank}:{level}: {outcome:?}");
            } else {
                assert_eq!(outcome, Err(CastError::Condition));
            }
        }
        assert!(cast_terms(&skill, 1, context(&calculator)).is_err());
        let mut inputs = context(&calculator);
        inputs.player_level = Some(15);
        assert!(cast_terms(&skill, 0, inputs).is_err());
        inputs.conditions.weapon_kind = "FIST";
        assert_eq!(cast_terms(&skill, 1, inputs), Err(CastError::Condition));
    }
    fn context(calc: &StatCalculator) -> CastStatContext<'_> {
        CastStatContext {
            player_level: None,
            calculator: calc,
            conditions: FunctionConditions {
                weapon_kind: "SWORD",
                can_resurrect: true,
            },
            attack_speed: 333,
            cast_speed: 333,
            flags: StatusFlags::default(),
        }
    }
    fn start(terms: CastTerms, tick: u64) -> CastStart {
        CastStart {
            now: Tick(tick),
            ordinal: Ordinal(1),
            incarnation: 1,
            terms,
            target: Some(binding(2)),
            affect_limit: 0,
            targets: vec![binding(2)],
        }
    }
    fn terms(animation: i64, continuous: bool) -> CastTerms {
        CastTerms {
            skill: SkillRef {
                id: SkillId(1177),
                rank: 1,
            },
            animation: FrozenRatio::new(ExactRatio::whole(animation)),
            continuous,
            mp_start: 3,
            mp_end: 7,
            hp_end: 1,
            reuse_ms: 1500,
        }
    }

    #[test]
    fn staging_obeys_receive_order_and_never_retargets_or_cancels_a_future_start() {
        let entry = |ordinal, caster| StagedCast {
            ordinal: Ordinal(ordinal),
            caster: binding(caster),
            skill_id: SkillId(3),
            target: Some(CapturedTarget {
                entity: binding(9).entity,
                incarnation: Some(2),
            }),
            canceled: None,
        };
        let mut queue = StagedActions::default();
        queue.stage(entry(1, 2)).unwrap();
        queue.stage(entry(2, 1)).unwrap();
        queue.cancel(binding(2).entity, Some(CastId(100)), CastCancelReason::Stopped);
        queue.cancel(binding(1).entity, None, CastCancelReason::Moved);
        queue.stage(entry(4, 1)).unwrap();
        assert_eq!(queue.stage(entry(3, 3)), Err(CastError::InvalidState));
        let out: Vec<_> = queue
            .drain()
            .filter_map(|e| {
                if let StagedAction::Cast(c) = e {
                    Some(c)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(out.iter().map(|e| e.ordinal.0).collect::<Vec<_>>(), [1, 2, 4]);
        assert_eq!(
            out.iter().map(|e| e.canceled).collect::<Vec<_>>(),
            [None, Some(CastCancelReason::Moved), None]
        );
        assert_eq!(out[2].target.unwrap().incarnation, Some(2));
        assert_eq!(queue.drain().count(), 0);
        let mut queue = StagedActions::default();
        queue.stage(entry(1, 1)).unwrap();
        queue.stage(entry(2, 1)).unwrap();
        queue.cancel(binding(1).entity, None, CastCancelReason::Stopped);
        queue.stage(entry(4, 1)).unwrap();
        let out: Vec<_> = queue
            .drain()
            .filter_map(|e| {
                if let StagedAction::Cast(c) = e {
                    Some(c)
                } else {
                    None
                }
            })
            .collect();
        assert!(out[..2]
            .iter()
            .all(|entry| entry.canceled == Some(CastCancelReason::Stopped)));
        assert_eq!(out[2].canceled, None);
    }

    #[test]
    fn canonical_cast_pays_once_per_stage_and_preserves_reuse_after_cancellation() {
        let mut sidecar = blank();
        let (mut next, mut mp) = (1, 10);
        let cast = sidecar
            .begin_cast(&mut next, &mut mp, 100, start(terms(550, false), 10))
            .unwrap();
        assert_eq!(
            (
                next,
                mp,
                cast.launch_at,
                cast.complete_at,
                cast.interrupt_until,
                cast.reuse_until
            ),
            (2, 7, Tick(12), Tick(16), Tick(13), Tick(25))
        );
        assert!(sidecar.cancel_cast(Some(CastId(2))).is_none());
        assert!(sidecar.cancel_cast(Some(cast.id)).is_some());
        assert!(sidecar.cancel_cast(None).is_none());
        assert_eq!(mp, 7);
        mp = 20;
        let before = sidecar.clone();
        assert_eq!(
            sidecar.begin_cast(&mut next, &mut mp, 100, start(terms(550, false), 24)),
            Err(CastError::SkillCooldown)
        );
        assert_eq!(sidecar, before);
        assert_eq!((mp, next), (20, 2));
        let cast = sidecar
            .begin_cast(&mut next, &mut mp, 100, start(terms(550, false), 25))
            .unwrap();
        assert_eq!(cast.id, CastId(2));
        assert_eq!(mp, 17);
    }

    #[test]
    fn rejected_start_is_atomic_including_counter_and_deadline_overflow() {
        for (hp, initial_mp, at, next_id, expected) in [
            (0, 10, 0, 1, CastError::DeadActor),
            (1, 10, 0, 1, CastError::InsufficientHp),
            (100, 9, 0, 1, CastError::InsufficientMp),
            (100, 10, u64::MAX, 1, CastError::InvalidState),
            (100, 10, 0, u64::MAX, CastError::InvalidState),
            (100, 10, 0, 0, CastError::InvalidState),
        ] {
            let mut sidecar = blank();
            let before = sidecar.clone();
            let (mut next, mut mp) = (next_id, initial_mp);
            assert_eq!(
                sidecar.begin_cast(&mut next, &mut mp, hp, start(terms(550, false), at)),
                Err(expected)
            );
            assert_eq!((sidecar, next, mp), (before, next_id, initial_mp));
        }
    }

    #[test]
    fn deadlines_and_live_indexes_survive_midcast_serialization_without_resampling() {
        for (animation, continuous, launch, impact, interrupt) in [
            (0, false, 0, 1, 0),
            (100, false, 0, 4, 0),
            (500, false, 1, 5, 3),
            (500, true, 0, 4, 3),
            (550, false, 2, 6, 3),
            (550, true, 0, 4, 3),
            (1200, false, 8, 12, 10),
            (1200, true, 5, 9, 10),
        ] {
            let mut sidecar = blank();
            let (mut next, mut mp) = (1, 20);
            let cast = sidecar
                .begin_cast(&mut next, &mut mp, 100, start(terms(animation, continuous), 10))
                .unwrap();
            assert_eq!(
                (cast.launch_at.0, cast.complete_at.0, cast.interrupt_until.0),
                (10 + launch, 10 + impact, 10 + interrupt)
            );
            let encoded = serde_json::to_vec(&sidecar).unwrap();
            let mut restored: Phase3Entity = serde_json::from_slice(&encoded).unwrap();
            let mut entities = BTreeMap::from([(binding(1).entity, restored.clone())]);
            let mut schedule = Phase3Schedule::rebuild(&entities).unwrap();
            assert_eq!(schedule.cast_launches(cast.launch_at).len(), 1);
            assert!(!restored.launch_cast(
                Tick(cast.launch_at.0.saturating_sub(1)),
                cast.id,
                |_| true
            ));
            assert!(restored.launch_cast(cast.launch_at, cast.id, |_| false));
            assert!(!restored.launch_cast(cast.launch_at, cast.id, |_| true));
            if let CastState::Casting(cast) = &restored.cast {
                assert!(cast.targets.is_empty());
            }
            entities.insert(binding(1).entity, restored.clone());
            schedule
                .refresh(binding(1).entity, Some(&restored))
                .unwrap();
            assert!(schedule.cast_launches(cast.launch_at).is_empty());
            assert_eq!(schedule.cast_impacts(cast.complete_at).len(), 1);
            assert_eq!(schedule, Phase3Schedule::rebuild(&entities).unwrap());
            restored.cancel_cast(None);
            schedule
                .refresh(binding(1).entity, Some(&restored))
                .unwrap();
            assert!(schedule.cast_impacts(Tick(u64::MAX)).is_empty());
        }
    }

    #[test]
    fn impact_cost_failure_keeps_source_payment_order_and_success_finalizes_after_handlers() {
        for (available_mp, hp, expected_mp, expected_hp, canceled) in [
            (6, 100, 6, 100, true),
            (7, 1, 0, 1, true),
            (10, 100, 3, 99, false),
        ] {
            let mut sidecar = blank();
            let (mut next, mut mp, mut hp) = (1, 20, hp);
            let cast = sidecar
                .begin_cast(&mut next, &mut mp, 100, start(terms(0, false), 10))
                .unwrap();
            assert!(sidecar.launch_cast(Tick(10), cast.id, |_| true));
            mp = available_mp;
            assert_eq!(
                sidecar.pay_cast_impact(Tick(10), cast.id, &mut mp, &mut hp, 7, 1),
                Ok(None)
            );
            let outcome = sidecar.pay_cast_impact(Tick(11), cast.id, &mut mp, &mut hp, 7, 1);
            assert_eq!((mp, hp), (expected_mp, expected_hp));
            if canceled {
                assert_eq!(outcome, Err(CastCancelReason::Resources));
                assert_eq!(sidecar.cast, CastState::Idle);
            } else {
                assert!(matches!(sidecar.cast, CastState::Casting(_)));
                assert_eq!(sidecar.finish_cast(cast.id), outcome.unwrap());
            }
            assert_eq!(
                sidecar.pay_cast_impact(Tick(11), cast.id, &mut mp, &mut hp, 7, 1),
                Ok(None)
            );
            assert_eq!(sidecar.reuse[&SkillId(1177)], Tick(25));
        }
    }

    #[test]
    fn target_limit_uses_source_width_and_ordinal_order_not_uuid_order() {
        let candidates = vec![
            AffectCandidate {
                ordinal: EntityOrdinal(2),
                binding: binding(1),
            },
            AffectCandidate {
                ordinal: EntityOrdinal(1),
                binding: binding(9),
            },
        ];
        let (limit, bindings) = collect_affect_targets(candidates.clone(), 100, 0, None).unwrap();
        assert_eq!(limit, 0);
        assert_eq!(bindings, [binding(9), binding(1)]);
        assert_eq!(
            collect_affect_targets(candidates.clone(), 1, 1, Some(0)).unwrap(),
            (1, vec![binding(9)])
        );
        assert!(collect_affect_targets(candidates.clone(), 1, 1, None).is_err());
        assert!(collect_affect_targets(candidates, 1, 0, Some(0)).is_err());
    }

    #[test]
    fn registry_costs_caps_flags_and_independent_interrupt_window_use_current_source_values() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let calculator = StatCalculator::default();
        // Every presently executable active rank, including isMagic4 references, has a
        // supported getter path. This fails if a future executable dance needs config.
        for skill in
            rules.skills().skills().values().filter(|s| {
                s.runtime_status == RuntimeSkillStatus::Executable && s.operate_type != "P"
            })
        {
            for rank in 1..=skill.max_level {
                assert!(
                    cast_costs(skill, rank, context(&calculator)).is_ok(),
                    "{} rank {rank}",
                    skill.id.0
                );
            }
        }
        let skill = rules.skills().get(SkillId(1177)).unwrap();
        let learned = vec![LearnedSkill {
            key: skill.key.clone(),
            level: 1,
        }];
        assert_eq!(
            learned_skill(rules.skills(), &learned, skill.id)
                .unwrap()
                .1
                .rank,
            1
        );
        assert_eq!(
            learned_skill(rules.skills(), &[], skill.id).unwrap_err(),
            CastError::SkillNotLearned
        );
        let base = cast_terms(skill, 1, context(&calculator)).unwrap();
        let multiplier = StatCalculator::try_new([StatFunction {
            key: FunctionKey {
                stat: "magicalMpConsumeRate".into(),
                priority: 20,
                install_sequence: 1,
                handler_ordinal: 0,
                function_ordinal: 1,
            },
            operation: FunctionOp::Mul(2 * Q),
            conditions: vec![],
        }])
        .unwrap();
        let changed = cast_costs(skill, 1, context(&multiplier)).unwrap();
        assert_eq!(changed.0, base.mp_start);
        assert_eq!(changed.1, base.mp_end * 2);
        let mut flags = context(&calculator);
        flags.flags = StatusFlags::ROOT;
        assert!(cast_terms(skill, 1, flags).is_ok());
        flags.flags = StatusFlags::STUN;
        assert_eq!(cast_terms(skill, 1, flags), Err(CastError::Incapacitated));
        flags.flags = StatusFlags::MAGIC_MUTE;
        assert_eq!(cast_terms(skill, 1, flags), Err(CastError::Silenced));
        for (at, success, expected) in [
            (12, true, true),
            (13, true, false),
            (14, true, false),
            (12, false, false),
        ] {
            let mut sidecar = blank();
            let (mut next, mut mp) = (1, 20);
            sidecar
                .begin_cast(&mut next, &mut mp, 100, start(terms(550, false), 10))
                .unwrap();
            assert_eq!(sidecar.interrupt_cast(Tick(at), success, skill).is_some(), expected);
        }
    }
}
