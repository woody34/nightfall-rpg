//! The replay log (Stories 3.2 and 3.4, plan §8 #2-#6; architecture.md §2.5).
//!
//! Each zone epoch is logged as: its start snapshot, then one [`AppliedTickRecord`] per tick
//! (appended by [`DurableTickGate`] and acknowledged before the tick is released), then a
//! completion [`Watermark`]. Replay opens an epoch with [`open_epoch`], which refuses one
//! without a watermark. Per-session audit frames go through [`SessionAuditWriter`].

mod audit;
mod codec;
mod codec_validation;
mod epoch;
mod gate;
mod port;
mod record;
mod snapshot_phase3;
mod snapshot_registry;
mod snapshot_value;

pub use audit::{SessionAuditWriter, AUDIT_BUFFER};
pub use codec::{
    command_admission_size, digest_record_upper_bound, validate_record_budget,
    AdmissionSizingError, CommandAdmissionSize, RECORD_SCHEMA_VERSION,
};
pub use epoch::{
    open_epoch, start_epoch, CheckedRecords, EpochStarted, RecordedEpoch, ReplayError,
};
pub use gate::{DurableTickGate, EpochProgress, GateConfig};
pub use port::{
    EventLog, NoReplayMetrics, RecordStream, RecoveryEpoch, ReplayLogMetrics, ZoneSnapshotRow,
    ZoneSnapshotStore,
};
pub use record::{
    decode_events, decode_outputs, decode_snapshot, encode_events, encode_outputs, encode_snapshot,
    phase3_snapshot_size, AppliedTickRecord, CodecError, EpochStatus, OutputForm, PlayerOutput,
    Seq, SessionInRecord, SessionOutRecord, StoredSnapshot, Watermark, WatermarkReason,
};
#[cfg(test)]
pub(crate) use record::{MAX_SNAPSHOT_ENCODED, MAX_SNAPSHOT_JSON};

/// Resolve immutable V6 inputs before domain restoration; historical epochs load only
/// their recorded inputs. A missing or mismatched compiled bundle is a hard error.
pub fn restore_snapshot(
    snapshot: crate::domain::zone::ZoneSnapshot,
) -> Result<crate::domain::zone::ZoneState, crate::domain::zone::SnapshotError> {
    use crate::domain::zone::{SnapshotError, ZoneState};
    let rules = snapshot
        .phase4
        .as_ref()
        .map(|p| {
            let skills = snapshot
                .phase3
                .as_ref()
                .ok_or(SnapshotError::CombatMismatch)?;
            crate::infrastructure::item_data::resolve_phase4_bundle(p.rules_hash, skills.rules_hash)
                .map(|bundle| bundle.items)
                .map_err(SnapshotError::Rules)
        })
        .transpose()?;
    ZoneState::from_snapshot_with_items(snapshot, rules)
}
