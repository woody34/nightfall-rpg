//! The replay log on NATS `JetStream`: streams `NF_ZONES` (snapshots, applied records,
//! watermarks) and `NF_SESSIONS` (audit frames), both file-backed with 7-day retention, created
//! or updated on startup. Every publish waits for the broker's ack; zone records carry a
//! `Nats-Msg-Id` so a retry is stored once. Publish latency goes to
//! `eventlog_publish_seconds{kind}`. A message (payload plus headers) may not exceed the
//! server's `max_payload`; [`EventLog::max_record_bytes`] reports the room left for a record.

use std::time::{Duration, Instant};

use async_nats::jetstream::consumer::{pull, DeliverPolicy};
use async_nats::jetstream::context::PublishAckFuture;
use async_nats::jetstream::message::PublishMessage;
use async_nats::jetstream::stream::{self, LastRawMessageErrorKind, StorageType};
use async_nats::jetstream::{self, Context};
use async_trait::async_trait;
use bytes::Bytes;
use opentelemetry::KeyValue;
use tokio_stream::StreamExt;

use super::{
    applied_msg_id, applied_subject, session_subject, snapshot_epoch, snapshot_subject,
    watermark_subject,
};
use crate::application::replay_log::{
    decode_snapshot, encode_snapshot, AppliedTickRecord, EpochStatus, EventLog, RecordStream, Seq,
    SessionInRecord, SessionOutRecord, StoredSnapshot, Watermark,
};
use crate::domain::zone::{ZoneId, ZoneSnapshot};
use crate::infrastructure::telemetry::Metrics;

/// Stream holding every zone epoch's snapshot, applied records and watermark.
pub const ZONES_STREAM: &str = "NF_ZONES";
/// Stream holding per-session audit frames.
pub const SESSIONS_STREAM: &str = "NF_SESSIONS";
/// Per-message retention in both streams. Initial snapshots can expire before later records;
/// recovery uses refreshed baselines and the durable database epoch index.
pub const RETENTION: Duration = Duration::from_hours(7 * 24);
/// Room kept below `max_payload` for the headers of a record message (`Nats-Msg-Id` is under
/// 100 bytes).
pub const HEADER_ROOM: usize = 1024;

/// The `JetStream` replay log.
#[derive(Clone)]
pub struct JetStreamEventLog {
    client: async_nats::Client,
    context: Context,
    zones: stream::Stream,
    metrics: Metrics,
}

impl JetStreamEventLog {
    /// Creates (or updates to this configuration) `NF_ZONES` and `NF_SESSIONS`.
    ///
    /// Fails if another stream claims overlapping subjects; `NF_EVENTS` must use
    /// `nightfall.*.*` (see `infrastructure::outbox`), not `nightfall.>`.
    pub async fn connect(client: async_nats::Client, metrics: Metrics) -> anyhow::Result<Self> {
        let context = jetstream::new(client.clone());
        let zones = ensure_stream(&context, ZONES_STREAM, "nightfall.zone.*.*.*").await?;
        ensure_stream(&context, SESSIONS_STREAM, "nightfall.session.*.*").await?;
        let zones = context
            .get_stream(zones)
            .await
            .map_err(|e| anyhow::anyhow!("open stream {ZONES_STREAM}: {e}"))?;
        Ok(Self {
            client,
            context,
            zones,
            metrics,
        })
    }

    async fn send(
        &self,
        subject: String,
        msg_id: Option<String>,
        payload: Vec<u8>,
    ) -> anyhow::Result<PublishAckFuture> {
        let mut msg = PublishMessage::build().payload(Bytes::from(payload));
        if let Some(id) = msg_id {
            msg = msg.message_id(id);
        }
        Ok(self.context.send_publish(subject, msg).await?)
    }

    /// Publishes and waits for the ack, timing the round trip.
    async fn publish(
        &self,
        kind: &'static str,
        subject: String,
        msg_id: Option<String>,
        payload: Vec<u8>,
    ) -> anyhow::Result<Seq> {
        let started = Instant::now();
        let ack = self.send(subject, msg_id, payload).await?.await?;
        self.record_latency(kind, started);
        if ack.duplicate {
            tracing::debug!(seq = ack.sequence, kind, "broker deduplicated replay-log publish");
        }
        Ok(Seq(ack.sequence))
    }

    fn record_latency(&self, kind: &'static str, started: Instant) {
        self.metrics
            .eventlog_publish_seconds
            .record(started.elapsed().as_secs_f64(), &[KeyValue::new("kind", kind)]);
    }

    /// Publishes a batch pipelined (all sends, then all acks). Returns how many were not
    /// acknowledged.
    async fn publish_batch(&self, kind: &'static str, batch: Vec<(String, Vec<u8>)>) -> u64 {
        if batch.is_empty() {
            return 0;
        }
        let started = Instant::now();
        let mut pending = Vec::with_capacity(batch.len());
        let mut failed = 0_u64;
        for (subject, payload) in batch {
            match self.send(subject, None, payload).await {
                Ok(ack) => pending.push(ack),
                Err(e) => {
                    tracing::debug!(error = %e, "audit publish failed");
                    failed = failed.saturating_add(1);
                },
            }
        }
        for ack in pending {
            if let Err(e) = ack.await {
                tracing::debug!(error = %e, "audit publish not acknowledged");
                failed = failed.saturating_add(1);
            }
        }
        self.record_latency(kind, started);
        failed
    }

    /// Messages currently stored on one literal subject.
    async fn count(&self, subject: &str) -> anyhow::Result<usize> {
        let mut subjects = self
            .zones
            .info_with_subjects(subject)
            .await
            .map_err(|e| anyhow::anyhow!("count {subject}: {e}"))?;
        let mut count = 0;
        while let Some(entry) = subjects.next().await {
            let (s, n) = entry.map_err(|e| anyhow::anyhow!("count {subject}: {e}"))?;
            if s == subject {
                count = n;
            }
        }
        Ok(count)
    }

    /// The payload and sequence of the last message on `subject`, if any.
    async fn last(&self, subject: &str) -> anyhow::Result<Option<(Seq, Bytes)>> {
        match self.zones.get_last_raw_message_by_subject(subject).await {
            Ok(m) => Ok(Some((Seq(m.sequence), m.payload))),
            Err(e) if e.kind() == LastRawMessageErrorKind::NoMessageFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("read last {subject}: {e}")),
        }
    }
}

async fn ensure_stream(context: &Context, name: &str, subject: &str) -> anyhow::Result<String> {
    context
        .create_or_update_stream(stream::Config {
            name: name.to_owned(),
            subjects: vec![subject.to_owned()],
            max_age: RETENTION,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|e| anyhow::anyhow!("create stream {name} ({subject}): {e}"))?;
    Ok(name.to_owned())
}

#[async_trait]
impl EventLog for JetStreamEventLog {
    fn max_record_bytes(&self) -> usize {
        // The limit the server last announced, so a reconnect to a server with another
        // `max_payload` is picked up on the next tick.
        self.client.max_payload().saturating_sub(HEADER_ROOM)
    }

    async fn append_applied(&self, record: &AppliedTickRecord) -> anyhow::Result<Seq> {
        self.publish(
            "applied",
            applied_subject(record.zone, record.epoch),
            Some(applied_msg_id(record.zone, record.epoch, record.tick)),
            record.encode(),
        )
        .await
    }

    async fn write_snapshot(&self, snapshot: &ZoneSnapshot) -> anyhow::Result<Seq> {
        let subject = snapshot_subject(snapshot.seed.zone, snapshot.seed.epoch);
        self.publish("snapshot", subject.clone(), Some(subject), encode_snapshot(snapshot)?)
            .await
    }

    async fn read_snapshot(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<Option<StoredSnapshot>> {
        self.last(&snapshot_subject(zone, epoch))
            .await?
            .map(|(seq, payload)| {
                Ok(StoredSnapshot {
                    seq,
                    snapshot: decode_snapshot(&payload)?,
                })
            })
            .transpose()
    }

    async fn write_recovery_snapshot(&self, snapshot: &ZoneSnapshot) -> anyhow::Result<Seq> {
        let subject =
            format!("nightfall.zone.{}.{}.recovery", snapshot.seed.zone.0, snapshot.seed.epoch);
        let payload = encode_snapshot(snapshot)?;
        let id = super::recovery_msg_id(&subject, snapshot.tick, &payload);
        self.publish("snapshot", subject, Some(id), payload).await
    }

    async fn read_recovery_snapshot(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<Option<StoredSnapshot>> {
        let subject = format!("nightfall.zone.{}.{epoch}.recovery", zone.0);
        if let Some((seq, bytes)) = self.last(&subject).await? {
            return Ok(Some(StoredSnapshot {
                seq,
                snapshot: decode_snapshot(&bytes)?,
            }));
        }
        self.read_snapshot(zone, epoch).await
    }

    async fn write_watermark(&self, watermark: &Watermark) -> anyhow::Result<Seq> {
        let subject = watermark_subject(watermark.zone, watermark.epoch);
        self.publish("watermark", subject.clone(), Some(subject), watermark.encode()?)
            .await
    }

    async fn epoch_status(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<EpochStatus> {
        if self.last(&snapshot_subject(zone, epoch)).await?.is_none() {
            return Ok(EpochStatus::Missing);
        }
        if let Some((_, payload)) = self.last(&watermark_subject(zone, epoch)).await? {
            return Ok(EpochStatus::Complete(Watermark::decode(&payload)?));
        }
        let last_tick = self
            .last(&applied_subject(zone, epoch))
            .await?
            .map(|(_, p)| AppliedTickRecord::decode(&p).map(|r| r.tick))
            .transpose()?;
        Ok(EpochStatus::Incomplete { last_tick })
    }

    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>> {
        let mut subjects = self
            .zones
            .info_with_subjects(format!("nightfall.zone.{}.*.snapshot", zone.0))
            .await
            .map_err(|e| anyhow::anyhow!("list snapshots of zone {}: {e}", zone.0))?;
        let mut latest = None;
        while let Some(entry) = subjects.next().await {
            let (subject, _count) = entry.map_err(|e| anyhow::anyhow!("list snapshots: {e}"))?;
            latest = latest.max(snapshot_epoch(zone, &subject));
        }
        Ok(latest)
    }

    async fn read_epoch(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<RecordStream> {
        let subject = applied_subject(zone, epoch);
        // Read the records present now; an ordered consumer never ends on its own, so the
        // stream takes exactly that many (and never waits for one more).
        let count = self.count(&subject).await?;
        if count == 0 {
            return Ok(Box::pin(tokio_stream::empty()));
        }
        let consumer = self
            .zones
            .create_consumer(pull::OrderedConfig {
                filter_subject: subject,
                deliver_policy: DeliverPolicy::All,
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!("open consumer: {e}"))?;
        let messages = consumer
            .messages()
            .await
            .map_err(|e| anyhow::anyhow!("read {ZONES_STREAM}: {e}"))?;
        let records = messages
            .map(|m| -> anyhow::Result<AppliedTickRecord> {
                let m = m.map_err(|e| anyhow::anyhow!("read {ZONES_STREAM}: {e}"))?;
                Ok(AppliedTickRecord::decode(&m.payload)?)
            })
            .take(count);
        Ok(Box::pin(records))
    }

    async fn append_session_in(&self, records: &[SessionInRecord]) -> anyhow::Result<u64> {
        Ok(self
            .publish_batch(
                "session_in",
                records
                    .iter()
                    .map(|r| (session_subject(r.session, "in"), r.encode()))
                    .collect(),
            )
            .await)
    }

    async fn append_session_out(&self, records: &[SessionOutRecord]) -> anyhow::Result<u64> {
        Ok(self
            .publish_batch(
                "session_out",
                records
                    .iter()
                    .map(|r| (session_subject(r.session, "out"), r.encode()))
                    .collect(),
            )
            .await)
    }
}
