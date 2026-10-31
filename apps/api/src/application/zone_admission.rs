//! Deterministic, pre-draft V5/V6 record admission. The codec supplies all byte arithmetic.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::replay_log::{
    command_admission_size, digest_record_upper_bound, validate_record_budget,
    AdmissionSizingError, CommandAdmissionSize,
};
use crate::domain::zone::{CommandSource, EntityId, SessionGeneration, ZoneCommand, ZoneInput};

/// Checked platform representation of the policy recorded in epoch metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordAdmissionPolicy {
    /// Version of this byte-admission algorithm.
    pub version: u32,
    /// Guaranteed digest-form record ceiling, including all framing.
    pub record_budget: usize,
    /// Maximum complete canonical encoded input, including retained admission state.
    pub max_command_bytes: usize,
    /// Maximum reserved recipients, including observers and command-response recipients.
    pub max_observers: usize,
}

impl Default for RecordAdmissionPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            record_budget: 1_048_576 - 4096,
            max_command_bytes: 262_144,
            max_observers: 1000,
        }
    }
}

impl TryFrom<crate::domain::zone::Phase3AdmissionPolicy> for RecordAdmissionPolicy {
    type Error = AdmissionSizingError;

    fn try_from(policy: crate::domain::zone::Phase3AdmissionPolicy) -> Result<Self, Self::Error> {
        let invalid = |_| AdmissionSizingError::InvalidBudget;
        if !policy.is_valid() {
            return Err(AdmissionSizingError::InvalidBudget);
        }
        Ok(Self {
            version: policy.version,
            record_budget: usize::try_from(policy.record_budget).map_err(invalid)?,
            max_command_bytes: usize::try_from(policy.max_command_bytes).map_err(invalid)?,
            max_observers: usize::try_from(policy.max_observers).map_err(invalid)?,
        })
    }
}

impl RecordAdmissionPolicy {
    /// Refuse an epoch that cannot fit one maximum legal input with the full reserve.
    pub fn validate(self, gate_ceiling: usize) -> Result<(), AdmissionSizingError> {
        if !matches!(self.version, 1 | 2)
            || self.max_command_bytes > 262_144
            || self.max_observers == 0
        {
            return Err(AdmissionSizingError::InvalidBudget);
        }
        validate_record_budget(
            self.record_budget,
            gate_ceiling,
            self.max_command_bytes,
            self.max_observers,
        )
    }

    pub(super) fn measure(
        self,
        input: &ZoneInput,
    ) -> Result<CommandAdmissionSize, AdmissionFailureReason> {
        let size = command_admission_size(input).map_err(|_| AdmissionFailureReason::Sizing)?;
        if size.normalized_bytes() > self.max_command_bytes {
            return Err(AdmissionFailureReason::CommandTooLarge);
        }
        Ok(size)
    }

    pub(super) fn fits(self, sizes: &[CommandAdmissionSize], recipients: usize) -> bool {
        recipients <= self.max_observers
            && digest_record_upper_bound(sizes, recipients)
                .is_ok_and(|bytes| bytes <= self.record_budget)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod policy_tests {
    #[test]
    fn v6_policy_retains_exact_byte_ceiling_proof_and_refuses_unknown_versions() {
        use super::*;
        let original = RecordAdmissionPolicy::default();
        let v6 = RecordAdmissionPolicy::try_from(
            crate::domain::zone::Phase3AdmissionPolicy::version_two(),
        )
        .expect("recorded V6 policy");
        assert_eq!(
            v6,
            RecordAdmissionPolicy {
                version: 2,
                ..original
            }
        );
        assert!(original.validate(1_044_480).is_ok());
        assert!(v6.validate(1_044_480).is_ok());
        assert!(v6.validate(1_044_479).is_err());
        assert!(RecordAdmissionPolicy {
            max_command_bytes: 262_145,
            ..v6
        }
        .validate(1_044_480)
        .is_err());
        for version in [0, 3, u32::MAX] {
            assert!(RecordAdmissionPolicy { version, ..v6 }
                .validate(1_044_480)
                .is_err());
        }
    }
}

/// Bounded classifications for terminal ingress failures; no gameplay rejection is emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionFailureReason {
    /// A single normalized command exceeds its own bound.
    CommandTooLarge,
    /// A single command cannot fit with framing and current recipients.
    RecordTooLarge,
    /// Configured recipient cardinality would be exceeded.
    TooManyRecipients,
    /// Codec byte arithmetic failed.
    Sizing,
}

/// Producer notification for a removed input. Contains no command body or credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionFailure {
    /// Session source (or system producer).
    pub source: CommandSource,
    /// Subject used to correlate lifecycle admissions.
    pub entity: Option<EntityId>,
    /// Lifecycle or session generation, when present.
    pub generation: Option<SessionGeneration>,
    /// Original intent sequence.
    pub seq: Option<u32>,
    /// Policy version used at selection.
    pub policy_version: u32,
    /// Measured complete canonical command length.
    pub normalized_bytes: usize,
    /// Bounded failure classification.
    pub reason: AdmissionFailureReason,
}

pub(super) fn reserve_recipients(recipients: &mut BTreeSet<EntityId>, input: &ZoneInput) {
    if let CommandSource::Session { entity, .. } = input.source {
        recipients.insert(entity);
    }
    match input.command {
        ZoneCommand::SpawnPlayer { entity, .. } | ZoneCommand::ReplaceSession { entity, .. } => {
            recipients.insert(entity);
        },
        ZoneCommand::CastSkill { .. }
        | ZoneCommand::CancelCast { .. }
        | ZoneCommand::AcceptResurrect { .. }
        | ZoneCommand::LearnSkill { .. }
        | ZoneCommand::MaterializePendingItems(_)
        | ZoneCommand::ExtendItemRange { .. }
        | ZoneCommand::Item(_)
        | ZoneCommand::Phase5(_)
        | ZoneCommand::ChangeClass { .. }
        | ZoneCommand::SpawnNpc { .. }
        | ZoneCommand::Despawn { .. }
        | ZoneCommand::MoveTo { .. }
        | ZoneCommand::SetTarget { .. }
        | ZoneCommand::Attack { .. }
        | ZoneCommand::StopAttack { .. }
        | ZoneCommand::Respawn { .. }
        | ZoneCommand::StopMove { .. }
        | ZoneCommand::AddAggro { .. } => {}, // No new observer. Despawns never reduce the reserve.
    }
}

pub(super) fn failure(
    input: &ZoneInput,
    policy: RecordAdmissionPolicy,
    reason: AdmissionFailureReason,
) -> AdmissionFailure {
    let generation = match input.source {
        CommandSource::Session { generation, .. } => Some(generation),
        CommandSource::System => match input.command {
            ZoneCommand::SpawnPlayer { generation, .. }
            | ZoneCommand::ReplaceSession { generation, .. } => Some(generation),
            ZoneCommand::CastSkill { .. }
            | ZoneCommand::CancelCast { .. }
            | ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::LearnSkill { .. }
            | ZoneCommand::MaterializePendingItems(_)
            | ZoneCommand::ExtendItemRange { .. }
            | ZoneCommand::Item(_)
            | ZoneCommand::Phase5(_)
            | ZoneCommand::ChangeClass { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::MoveTo { .. }
            | ZoneCommand::SetTarget { .. }
            | ZoneCommand::Attack { .. }
            | ZoneCommand::StopAttack { .. }
            | ZoneCommand::Respawn { .. }
            | ZoneCommand::StopMove { .. }
            | ZoneCommand::AddAggro { .. } => None,
        },
    };
    AdmissionFailure {
        source: input.source,
        entity: input.command.entity(),
        generation,
        seq: input.seq,
        policy_version: policy.version,
        normalized_bytes: command_admission_size(input)
            .map_or(usize::MAX, CommandAdmissionSize::normalized_bytes),
        reason,
    }
}
