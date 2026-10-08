//! The replay log (Stories 3.2 and 3.4, plan §8 #2-#6; architecture.md §2.5).
//!
//! Each zone epoch is logged as: its start snapshot, then one [`AppliedTickRecord`] per tick
//! (appended by [`DurableTickGate`] and acknowledged before the tick is released), then a
//! completion [`Watermark`]. Replay opens an epoch with [`open_epoch`], which refuses one
//! without a watermark. Per-session audit frames go through [`SessionAuditWriter`].

mod audit;
mod codec;
mod epoch;
mod gate;
mod port;
mod record;

pub use audit::{SessionAuditWriter, AUDIT_BUFFER};
pub use epoch::{
    open_epoch, start_epoch, CheckedRecords, EpochStarted, RecordedEpoch, ReplayError,
};
pub use gate::{DurableTickGate, EpochProgress, GateConfig};
pub use port::{
    EventLog, NoReplayMetrics, RecordStream, ReplayLogMetrics, ZoneSnapshotRow, ZoneSnapshotStore,
};
pub use record::{
    decode_outputs, decode_snapshot, encode_outputs, encode_snapshot, AppliedTickRecord,
    CodecError, EpochStatus, PlayerOutput, Seq, SessionInRecord, SessionOutRecord, StoredSnapshot,
    Watermark, WatermarkReason,
};
