//! The ports the replay log needs: the log itself, the Postgres snapshot index, and the
//! handful of metrics the application-side policy emits.

use std::pin::Pin;

use async_trait::async_trait;
use tokio_stream::Stream;

use super::record::{
    AppliedTickRecord, EpochStatus, Seq, SessionInRecord, SessionOutRecord, StoredSnapshot,
    Watermark,
};
use crate::domain::zone::{ZoneId, ZoneSnapshot};

/// An epoch's applied-tick records in log order.
pub type RecordStream = Pin<Box<dyn Stream<Item = anyhow::Result<AppliedTickRecord>> + Send>>;

/// The replay log (plan §8 #2, #3, #5; architecture.md §2.5). The zone actor, through its
/// `DurableTickGate`, is the single writer of an epoch's applied records.
///
/// Every `append_applied`, `write_snapshot` and `write_watermark` returns only once the
/// record is durable (the `JetStream` ack). Appending the same `(zone, epoch, tick)` twice
/// stores it once (broker-side dedupe on `Nats-Msg-Id`).
#[async_trait]
pub trait EventLog: Send + Sync {
    /// The largest encoded [`AppliedTickRecord`] one append can store (`JetStream`: the
    /// server's `max_payload` less room for headers). A larger append fails every time, so
    /// the gate shrinks such a record with [`AppliedTickRecord::bounded`] first.
    fn max_record_bytes(&self) -> usize;

    /// Appends one tick's record. Durable and acknowledged.
    async fn append_applied(&self, record: &AppliedTickRecord) -> anyhow::Result<Seq>;

    /// Writes an epoch's start snapshot (`snapshot.seed` names zone and epoch). Must precede
    /// the epoch's first applied record; `EpochLog::start` is the only caller and enforces it.
    async fn write_snapshot(&self, snapshot: &ZoneSnapshot) -> anyhow::Result<Seq>;

    /// The epoch's start snapshot, if it is in the log.
    async fn read_snapshot(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<Option<StoredSnapshot>>;

    /// Closes an epoch. Written after the zone actor has stopped.
    async fn write_watermark(&self, watermark: &Watermark) -> anyhow::Result<Seq>;

    /// Whether the epoch is replayable.
    async fn epoch_status(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<EpochStatus>;

    /// The highest epoch of `zone` with a snapshot in the log.
    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>>;

    /// The epoch's applied records in log order, as stored at the time of the call. Does not
    /// check completeness or contiguity; replay goes through `open_epoch`, which does.
    async fn read_epoch(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<RecordStream>;

    /// Appends inbound audit frames, in order per session. Best effort: returns how many were
    /// not acknowledged (the caller counts them as dropped).
    async fn append_session_in(&self, records: &[SessionInRecord]) -> anyhow::Result<u64>;

    /// Appends outbound audit frames, in order per session. Best effort, as above.
    async fn append_session_out(&self, records: &[SessionOutRecord]) -> anyhow::Result<u64>;
}

/// One row of `zone_snapshots`: the epoch-start snapshot bytes (identical to the log's) plus
/// where the epoch lives in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneSnapshotRow {
    /// The zone.
    pub zone: ZoneId,
    /// The epoch.
    pub epoch: u64,
    /// `encode_snapshot` of the epoch-start snapshot.
    pub snapshot: Vec<u8>,
    /// Log position of the snapshot message.
    pub snapshot_seq: Seq,
    /// Log position of the epoch's first applied record, once acknowledged.
    pub first_seq: Option<Seq>,
    /// Wall-clock time of tick 0, Unix ms.
    pub time_origin_ms: i64,
    /// Server build that ran the epoch.
    pub build_id: String,
    /// Hash of the zone configuration.
    pub config_hash: String,
    /// Snapshot layout version.
    pub schema_version: u32,
}

/// The Postgres index of epochs (`zone_snapshots`). Lets operators and the replay tool find an
/// epoch without scanning the stream.
#[async_trait]
pub trait ZoneSnapshotStore: Send + Sync {
    /// Inserts the row. Inserting the same `(zone, epoch)` again is a no-op.
    async fn insert(&self, row: &ZoneSnapshotRow) -> anyhow::Result<()>;

    /// Records the log position of the epoch's first applied record.
    async fn set_first_seq(&self, zone: ZoneId, epoch: u64, seq: Seq) -> anyhow::Result<()>;

    /// One epoch's row.
    async fn get(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<Option<ZoneSnapshotRow>>;

    /// The highest epoch recorded for `zone`.
    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>>;
}

/// The metrics the replay-log policy emits. The telemetry catalogue implements it; tests use
/// [`NoReplayMetrics`]. Publish latency is recorded by the log adapter itself.
pub trait ReplayLogMetrics: Send + Sync {
    /// One failed attempt to append an applied record (the zone is stalling).
    fn append_failed(&self) {}

    /// One tick's record was too large for the log and was stored with its outputs as
    /// SHA-256 digests.
    fn record_digested(&self) {}

    /// The zone entered (`true`) or left (`false`) the paused state.
    fn zone_paused(&self, _paused: bool) {}

    /// Audit frames that were dropped (buffer full or not acknowledged).
    fn audit_dropped(&self, _count: u64) {}
}

/// Records nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoReplayMetrics;

impl ReplayLogMetrics for NoReplayMetrics {}
