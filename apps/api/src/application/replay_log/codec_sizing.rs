//! F2 sizing uses the actual hand-authored protobuf messages. No simulation, RNG or clocks.
//! Unknown application ordinals/ticks reserve full-width varints; normalized data stays exact.

use bytes::Bytes;
use prost::Message;
use thiserror::Error;

use super::{command_to_pb, source_to_pb, PbDisposition, PbPlayerOutput, PbRecord};
use crate::domain::zone::{AppliedCommand, Ordinal, ZoneInput};

/// Per-command proof, computed only from the complete normalized command and source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandAdmissionSize {
    normalized_bytes: usize,
    record_contribution: usize,
}
impl CommandAdmissionSize {
    /// Canonical `PbCommand` bytes without the not-yet-assigned ordinal or record wrapper.
    /// Includes the complete load/progression/strings, source/generation and optional seq.
    #[must_use]
    pub const fn normalized_bytes(self) -> usize {
        self.normalized_bytes
    }
    /// Record tag/length wrapper, worst-width ordinal and one worst legal disposition.
    #[must_use]
    pub const fn record_contribution(self) -> usize {
        self.record_contribution
    }
}

/// Sizing/configuration failures are terminal admission failures, never tick rejections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AdmissionSizingError {
    /// A byte count exceeded the platform representation.
    #[error("record admission byte count overflow")]
    Overflow,
    /// A policy cannot fit its mandatory framing, recipient reserve and maximum command.
    #[error("record admission policy cannot fit one maximum command")]
    InvalidBudget,
}

/// Measure a complete normalized command with the same encoder used by record production.
/// The 256-KiB limit applies to `normalized_bytes`; `record_contribution` also reserves the
/// application ordinal, nested prefixes and disposition. No field/list/string is truncated.
pub fn command_admission_size(
    input: &ZoneInput,
) -> Result<CommandAdmissionSize, AdmissionSizingError> {
    let mut command = command_to_pb(&AppliedCommand {
        ordinal: Ordinal(0),
        source: input.source,
        seq: input.seq,
        command: input.command.clone(),
    });
    let normalized_bytes = command.encoded_len();
    command.ordinal = u64::MAX;
    let disposition = PbDisposition {
        ordinal: u64::MAX,
        session: source_to_pb(input.source),
        seq: input.seq,
        tick_seen: u64::MAX,
        // All existing and pinned Phase 3 reasons fit one positive varint byte. Reserving
        // the entire 1..127 band also covers the specified new reasons starting at21.
        reason: 127,
    };
    let record_contribution = framed(command.encoded_len())?
        .checked_add(framed(disposition.encoded_len())?)
        .ok_or(AdmissionSizingError::Overflow)?;
    Ok(CommandAdmissionSize {
        normalized_bytes,
        record_contribution,
    })
}

/// Upper bound for the guaranteed digest-form record. recipients must reserve the union
/// of current observers, candidate spawns/replacements and response recipients, without
/// subtracting removals. Repeated field prefixes are measured for each individual entry.
pub fn digest_record_upper_bound(
    commands: &[CommandAdmissionSize],
    recipients: usize,
) -> Result<usize, AdmissionSizingError> {
    let mut total = framing_bytes()
        .checked_add(
            output_entry_bytes()?
                .checked_mul(recipients)
                .ok_or(AdmissionSizingError::Overflow)?,
        )
        .ok_or(AdmissionSizingError::Overflow)?;
    for command in commands {
        total = total
            .checked_add(command.record_contribution)
            .ok_or(AdmissionSizingError::Overflow)?;
    }
    Ok(total)
}

/// Validate a proposed epoch policy against its actual gate ceiling. `max_recipients` is the
/// complete configured observer AND possible extra response reserve. The caller must make
/// that distinction explicit rather than assuming a queue cardinality bounds output size.
pub fn validate_record_budget(
    record_budget: usize,
    gate_ceiling: usize,
    max_command_bytes: usize,
    max_recipients: usize,
) -> Result<(), AdmissionSizingError> {
    if record_budget == 0 || record_budget > gate_ceiling || max_command_bytes == 0 {
        return Err(AdmissionSizingError::InvalidBudget);
    }
    // Maximum optional session/generation/seq and full application ordinal/tick fields.
    let disposition = PbDisposition {
        ordinal: u64::MAX,
        session: Some(super::PbSession {
            entity: vec![0; 16],
            generation: u64::MAX,
        }),
        seq: Some(u32::MAX),
        tick_seen: u64::MAX,
        reason: 127,
    };
    let maximum = CommandAdmissionSize {
        normalized_bytes: max_command_bytes,
        // uint64 ordinal occupies its one-byte tag plus ten-byte varint.
        record_contribution: framed(
            max_command_bytes
                .checked_add(11)
                .ok_or(AdmissionSizingError::Overflow)?,
        )?
        .checked_add(framed(disposition.encoded_len())?)
        .ok_or(AdmissionSizingError::Overflow)?,
    };
    if digest_record_upper_bound(&[maximum], max_recipients)? > record_budget {
        return Err(AdmissionSizingError::InvalidBudget);
    }
    Ok(())
}

fn framing_bytes() -> usize {
    PbRecord {
        zone: u32::MAX,
        epoch: u64::MAX,
        tick: u64::MAX,
        server_time_ms: i64::MIN,
        commands: Vec::new(),
        dispositions: Vec::new(),
        outputs: Vec::new(),
        output_form: 1,
        events: Bytes::from_static(&[0; 32]),
        state_digest: Bytes::from_static(&[0; 32]),
        schema: 7,
    }
    .encoded_len()
}
fn output_entry_bytes() -> Result<usize, AdmissionSizingError> {
    framed(
        PbPlayerOutput {
            entity: vec![0; 16],
            encoded: Bytes::new(),
            sha256: Bytes::from_static(&[0; 32]),
        }
        .encoded_len(),
    )
}
fn framed(body: usize) -> Result<usize, AdmissionSizingError> {
    // Record command/disposition/output fields are tags5,6,7, each a one-byte wire tag.
    body.checked_add(1)
        .and_then(|value| value.checked_add(prost::length_delimiter_len(body)))
        .ok_or(AdmissionSizingError::Overflow)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use crate::application::replay_log::{AppliedTickRecord, OutputForm, PlayerOutput};
    use crate::domain::zone::{
        Disposition, EntityId, RejectReason, SessionGeneration, StateDigestVersion, Tick,
        ZoneCommand, ZoneId,
    };
    use uuid::Uuid;

    fn input() -> ZoneInput {
        let entity = EntityId::from_uuid(Uuid::from_u128(1));
        ZoneInput::session(
            entity,
            SessionGeneration(u64::MAX),
            u32::MAX,
            ZoneCommand::SetTarget {
                entity,
                target: None,
            },
        )
    }
    #[test]
    fn maximum_width_bound_matches_actual_digest_record_including_nested_prefixes() {
        let input = input();
        let size = command_admission_size(&input).unwrap();
        for recipients in [0, 1, 127, 128, 1000] {
            let record = AppliedTickRecord {
                zone: ZoneId(u32::MAX),
                epoch: u64::MAX,
                tick: Tick(u64::MAX),
                server_time_ms: i64::MIN,
                commands: vec![AppliedCommand {
                    ordinal: Ordinal(u64::MAX),
                    source: input.source,
                    seq: input.seq,
                    command: input.command.clone(),
                }],
                dispositions: vec![Disposition {
                    ordinal: Ordinal(u64::MAX),
                    source: input.source,
                    seq: input.seq,
                    tick_seen: Tick(u64::MAX),
                    reason: RejectReason::InvalidLoad,
                }],
                outputs: (0..recipients)
                    .map(|n| PlayerOutput {
                        entity: EntityId::from_uuid(Uuid::from_u128(u128::try_from(n).unwrap())),
                        bytes: Bytes::from_static(&[0; 32]),
                    })
                    .collect(),
                output_form: OutputForm::Sha256,
                events: Bytes::from_static(&[0; 32]),
                state_digest: Bytes::from_static(&[0; 32]),
                digest_version: StateDigestVersion::BinaryV5,
            };
            assert_eq!(
                digest_record_upper_bound(&[size], recipients).unwrap(),
                record.encoded_len()
            );
            assert_eq!(record.encoded_len(), record.encode().len());
        }
    }
    #[test]
    fn complete_command_lengths_cross_prefix_boundaries_without_truncation() {
        let mut input = input();
        for length in [1, 127, 128, 16_383, 16_384, 262_143, 262_144, 262_145] {
            input.command = ZoneCommand::SpawnPlayer {
                entity: EntityId::from_uuid(Uuid::nil()),
                name: "x".repeat(length),
                pos: crate::domain::zone::Vec2Fixed::from_tiles(0, 0),
                speed: crate::domain::zone::Speed::from_milli_tiles_per_tick(1),
                generation: SessionGeneration(1),
                load: None,
            };
            let size = command_admission_size(&input).unwrap();
            let canonical = command_to_pb(&AppliedCommand {
                ordinal: Ordinal(0),
                source: input.source,
                seq: input.seq,
                command: input.command.clone(),
            });
            assert_eq!(size.normalized_bytes(), canonical.encode_to_vec().len());
            assert!(size.normalized_bytes() > length);
            assert!(size.record_contribution() > size.normalized_bytes());
        }
    }
    #[test]
    fn budget_edges_and_overflow_fail_before_simulation() {
        let input = input();
        let size = command_admission_size(&input).unwrap();
        let maximum = CommandAdmissionSize {
            normalized_bytes: size.normalized_bytes(),
            record_contribution: size.record_contribution(),
        };
        let budget = digest_record_upper_bound(&[maximum], 1000).unwrap();
        // This input already has maximal source/seq widths, so the policy witness is exact.
        assert_eq!(
            validate_record_budget(budget - 1, budget + 1, maximum.normalized_bytes(), 1000),
            Err(AdmissionSizingError::InvalidBudget)
        );
        validate_record_budget(budget, budget + 1, maximum.normalized_bytes(), 1000).unwrap();
        validate_record_budget(budget + 1, budget + 1, maximum.normalized_bytes(), 1000).unwrap();
        assert_eq!(
            validate_record_budget(budget + 1, budget, maximum.normalized_bytes(), 1000),
            Err(AdmissionSizingError::InvalidBudget)
        );
        assert!(digest_record_upper_bound(&[], usize::MAX).is_err());
        assert!(validate_record_budget(usize::MAX, usize::MAX, usize::MAX, 0).is_err());
        assert!(digest_record_upper_bound(&[size, size], 1000).unwrap() > budget);
    }
}
