//! In-memory replay log and snapshot index, for tests and dependency-free dev. Behaves like
//! the `JetStream` adapter: one sequence across all subjects, records stored encoded, and
//! dedupe on the same message ids. Grows without bound; never use it in production.

use std::collections::BTreeMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::{
    applied_msg_id, applied_subject, session_subject, snapshot_subject, watermark_subject,
    HEADER_ROOM,
};
use crate::application::replay_log::{
    decode_snapshot, encode_snapshot, AppliedTickRecord, EpochStatus, EventLog, RecordStream, Seq,
    SessionInRecord, SessionOutRecord, StoredSnapshot, Watermark, ZoneSnapshotRow,
    ZoneSnapshotStore,
};
use crate::domain::zone::{ZoneId, ZoneSnapshot};

#[derive(Default)]
struct Log {
    /// Every stored message as `(seq, subject, payload)`, in sequence order.
    messages: Vec<(Seq, String, Vec<u8>)>,
    /// `Nats-Msg-Id` -> sequence, for dedupe.
    ids: BTreeMap<String, Seq>,
    next: u64,
}

impl Log {
    fn append(&mut self, subject: String, msg_id: Option<String>, payload: Vec<u8>) -> Seq {
        if let Some(seq) = msg_id.as_ref().and_then(|id| self.ids.get(id)) {
            return *seq;
        }
        self.next = self.next.saturating_add(1);
        let seq = Seq(self.next);
        if let Some(id) = msg_id {
            self.ids.insert(id, seq);
        }
        self.messages.push((seq, subject, payload));
        seq
    }

    fn last(&self, subject: &str) -> Option<&(Seq, String, Vec<u8>)> {
        self.messages.iter().rev().find(|(_, s, _)| s == subject)
    }
}

/// The replay log in a `Vec`.
pub struct InMemoryEventLog {
    log: Mutex<Log>,
    max_record_bytes: usize,
}

impl Default for InMemoryEventLog {
    /// Accepts records up to the size `JetStream` would with its default 1 MiB `max_payload`.
    fn default() -> Self {
        Self::with_max_record_bytes(DEFAULT_MAX_PAYLOAD.saturating_sub(HEADER_ROOM))
    }
}

/// NATS' default `max_payload`.
const DEFAULT_MAX_PAYLOAD: usize = 1024 * 1024;

impl InMemoryEventLog {
    /// A log that, like a broker with a smaller `max_payload`, refuses applied records whose
    /// encoding is longer than `max_record_bytes`.
    #[must_use]
    pub fn with_max_record_bytes(max_record_bytes: usize) -> Self {
        Self {
            log: Mutex::default(),
            max_record_bytes,
        }
    }

    /// Deletes a retained message to simulate stream expiry in recovery tests.
    pub fn delete_message(&self, seq: Seq) {
        self.log.lock().messages.retain(|(s, _, _)| *s != seq);
    }

    /// Every stored message's sequence and subject, in order. For ordering assertions.
    #[must_use]
    pub fn subjects(&self) -> Vec<(Seq, String)> {
        self.log
            .lock()
            .messages
            .iter()
            .map(|(seq, s, _)| (*seq, s.clone()))
            .collect()
    }

    /// Every stored inbound audit frame, in order.
    #[must_use]
    pub fn session_in(&self) -> Vec<SessionInRecord> {
        self.decoded(".in", |b| SessionInRecord::decode(b).ok())
    }

    /// Every stored outbound audit frame, in order.
    #[must_use]
    pub fn session_out(&self) -> Vec<SessionOutRecord> {
        self.decoded(".out", |b| SessionOutRecord::decode(b).ok())
    }

    fn decoded<T>(&self, suffix: &str, f: impl Fn(&[u8]) -> Option<T>) -> Vec<T> {
        self.log
            .lock()
            .messages
            .iter()
            .filter(|(_, s, _)| s.starts_with("nightfall.session.") && s.ends_with(suffix))
            .filter_map(|(_, _, p)| f(p))
            .collect()
    }
}

#[async_trait]
impl EventLog for InMemoryEventLog {
    fn max_record_bytes(&self) -> usize {
        self.max_record_bytes
    }

    async fn append_applied(&self, record: &AppliedTickRecord) -> anyhow::Result<Seq> {
        let payload = record.encode();
        anyhow::ensure!(
            payload.len() <= self.max_record_bytes,
            "record of {} bytes exceeds the limit of {}",
            payload.len(),
            self.max_record_bytes
        );
        Ok(self.log.lock().append(
            applied_subject(record.zone, record.epoch),
            Some(applied_msg_id(record.zone, record.epoch, record.tick)),
            payload,
        ))
    }

    async fn write_snapshot(&self, snapshot: &ZoneSnapshot) -> anyhow::Result<Seq> {
        let (zone, epoch) = (snapshot.seed.zone, snapshot.seed.epoch);
        let subject = snapshot_subject(zone, epoch);
        Ok(self
            .log
            .lock()
            .append(subject.clone(), Some(subject), encode_snapshot(snapshot)?))
    }

    async fn read_snapshot(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<Option<StoredSnapshot>> {
        let log = self.log.lock();
        log.last(&snapshot_subject(zone, epoch))
            .map(|(seq, _, payload)| {
                Ok(StoredSnapshot {
                    seq: *seq,
                    snapshot: decode_snapshot(payload)?,
                })
            })
            .transpose()
    }

    async fn write_recovery_snapshot(&self, snapshot: &ZoneSnapshot) -> anyhow::Result<Seq> {
        let subject =
            format!("nightfall.zone.{}.{}.recovery", snapshot.seed.zone.0, snapshot.seed.epoch);
        let payload = encode_snapshot(snapshot)?;
        let id = super::recovery_msg_id(&subject, snapshot.tick, &payload);
        Ok(self.log.lock().append(subject, Some(id), payload))
    }

    async fn read_recovery_snapshot(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<Option<StoredSnapshot>> {
        let subject = format!("nightfall.zone.{}.{epoch}.recovery", zone.0);
        let stored = self
            .log
            .lock()
            .last(&subject)
            .map(|(seq, _, bytes)| {
                Ok::<_, anyhow::Error>(StoredSnapshot {
                    seq: *seq,
                    snapshot: decode_snapshot(bytes)?,
                })
            })
            .transpose()?;
        match stored {
            Some(stored) => Ok(Some(stored)),
            None => self.read_snapshot(zone, epoch).await,
        }
    }

    async fn write_watermark(&self, watermark: &Watermark) -> anyhow::Result<Seq> {
        let subject = watermark_subject(watermark.zone, watermark.epoch);
        Ok(self
            .log
            .lock()
            .append(subject.clone(), Some(subject), watermark.encode()?))
    }

    async fn epoch_status(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<EpochStatus> {
        let log = self.log.lock();
        if log.last(&snapshot_subject(zone, epoch)).is_none() {
            return Ok(EpochStatus::Missing);
        }
        if let Some((_, _, payload)) = log.last(&watermark_subject(zone, epoch)) {
            return Ok(EpochStatus::Complete(Watermark::decode(payload)?));
        }
        let last_tick = log
            .last(&applied_subject(zone, epoch))
            .map(|(_, _, p)| AppliedTickRecord::decode(p).map(|r| r.tick))
            .transpose()?;
        Ok(EpochStatus::Incomplete { last_tick })
    }

    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>> {
        let log = self.log.lock();
        Ok(log
            .messages
            .iter()
            .filter_map(|(_, s, _)| super::snapshot_epoch(zone, s))
            .max())
    }

    async fn read_epoch(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<RecordStream> {
        let subject = applied_subject(zone, epoch);
        let records: Vec<anyhow::Result<AppliedTickRecord>> = self
            .log
            .lock()
            .messages
            .iter()
            .filter(|(_, s, _)| *s == subject)
            .map(|(_, _, p)| AppliedTickRecord::decode(p).map_err(anyhow::Error::from))
            .collect();
        Ok(Box::pin(tokio_stream::iter(records)))
    }

    async fn append_session_in(&self, records: &[SessionInRecord]) -> anyhow::Result<u64> {
        let mut log = self.log.lock();
        for r in records {
            log.append(session_subject(r.session, "in"), None, r.encode());
        }
        Ok(0)
    }

    async fn append_session_out(&self, records: &[SessionOutRecord]) -> anyhow::Result<u64> {
        let mut log = self.log.lock();
        for r in records {
            log.append(session_subject(r.session, "out"), None, r.encode());
        }
        Ok(0)
    }
}

/// The `zone_snapshots` index in a map.
#[derive(Default)]
pub struct InMemoryZoneSnapshotStore {
    rows: Mutex<BTreeMap<(ZoneId, u64), ZoneSnapshotRow>>,
    epochs: Mutex<BTreeMap<(ZoneId, u64), (Option<crate::domain::zone::Tick>, bool)>>,
}

#[async_trait]
impl ZoneSnapshotStore for InMemoryZoneSnapshotStore {
    async fn unresolved(
        &self,
        zone: ZoneId,
    ) -> anyhow::Result<Vec<crate::application::replay_log::RecoveryEpoch>> {
        Ok(self
            .epochs
            .lock()
            .iter()
            .filter(|((z, _), (_, closed))| *z == zone && !closed)
            .map(|((_, epoch), (tick, _))| crate::application::replay_log::RecoveryEpoch {
                epoch: *epoch,
                last_recorded_tick: *tick,
            })
            .collect())
    }
    async fn recording(
        &self,
        zone: ZoneId,
        epoch: u64,
        tick: crate::domain::zone::Tick,
    ) -> anyhow::Result<()> {
        self.epochs.lock().entry((zone, epoch)).or_default().0 = Some(tick);
        Ok(())
    }
    async fn checkpointed(
        &self,
        _zone: ZoneId,
        _epoch: u64,
        _tick: crate::domain::zone::Tick,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn close(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<()> {
        self.epochs.lock().entry((zone, epoch)).or_default().1 = true;
        Ok(())
    }

    async fn insert(&self, row: &ZoneSnapshotRow) -> anyhow::Result<()> {
        self.epochs.lock().entry((row.zone, row.epoch)).or_default();
        self.rows
            .lock()
            .entry((row.zone, row.epoch))
            .and_modify(|stored| {
                stored.snapshot.clone_from(&row.snapshot);
                stored.snapshot_seq = row.snapshot_seq;
                stored.schema_version = row.schema_version;
            })
            .or_insert_with(|| row.clone());
        Ok(())
    }

    async fn set_first_seq(&self, zone: ZoneId, epoch: u64, seq: Seq) -> anyhow::Result<()> {
        if let Some(row) = self.rows.lock().get_mut(&(zone, epoch)) {
            row.first_seq = Some(seq);
        }
        Ok(())
    }

    async fn get(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<Option<ZoneSnapshotRow>> {
        Ok(self.rows.lock().get(&(zone, epoch)).cloned())
    }

    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>> {
        Ok(self
            .rows
            .lock()
            .keys()
            .filter(|(z, _)| *z == zone)
            .map(|(_, e)| *e)
            .max())
    }
}
