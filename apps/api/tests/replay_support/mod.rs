//! Shared helpers for the replay-log tests: a fault-injecting `EventLog` wrapper (broker down,
//! slow acks, hang from a tick on), a fixed clock, and the fixture zone.

#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use nightfall_api::application::replay_log::{
    AppliedTickRecord, EpochStatus, EventLog, RecordStream, Seq, SessionInRecord, SessionOutRecord,
    StoredSnapshot, Watermark,
};
use nightfall_api::application::zone_bootstrap::ZoneDefinition;
use nightfall_api::application::Clock;
use nightfall_api::domain::zone::{ZoneId, ZoneSnapshot};
use nightfall_api::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};
use tokio::sync::Semaphore;

/// 2026-10-07T00:00:00Z.
pub const ORIGIN_MS: i64 = 1_791_331_200_000;

pub struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(ORIGIN_MS).unwrap()
    }
}

/// The fixture zone under another id, so tests sharing a broker do not collide.
pub fn zone_def(zone: u32) -> ZoneDefinition {
    let mut def = parse_zone(TEST_ZONE_TOML).unwrap();
    def.zone = ZoneId(zone);
    def
}

/// A zone id no other test run uses.
pub fn unique_zone() -> u32 {
    let n = uuid::Uuid::now_v7().as_u128();
    // Low bits of a v7 UUID are random.
    u32::try_from(n & 0x7fff_ffff).unwrap()
}

/// Wraps a log and injects faults into `append_applied`.
pub struct FaultyLog {
    pub inner: Arc<dyn EventLog>,
    /// Every append fails, as when the broker is unreachable.
    pub down: AtomicBool,
    /// Appends wait for a permit, so a test controls exactly when each ack arrives.
    pub manual_acks: AtomicBool,
    pub acks: Semaphore,
    /// Appends for this tick and later never return (a process stuck mid-tick).
    pub hang_from_tick: AtomicU64,
    pub attempts: AtomicU64,
    /// Nonempty session audit appends never acknowledge (shutdown timeout injection).
    pub hang_audit: AtomicBool,
}

impl FaultyLog {
    pub fn new(inner: Arc<dyn EventLog>) -> Self {
        Self {
            inner,
            down: AtomicBool::new(false),
            manual_acks: AtomicBool::new(false),
            acks: Semaphore::new(0),
            hang_from_tick: AtomicU64::new(u64::MAX),
            attempts: AtomicU64::new(0),
            hang_audit: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl EventLog for FaultyLog {
    fn max_record_bytes(&self) -> usize {
        self.inner.max_record_bytes()
    }

    async fn append_applied(&self, record: &AppliedTickRecord) -> anyhow::Result<Seq> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if record.tick.0 >= self.hang_from_tick.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.manual_acks.load(Ordering::SeqCst) {
            self.acks.acquire().await.unwrap().forget();
        }
        if self.down.load(Ordering::SeqCst) {
            anyhow::bail!("broker unavailable (injected)");
        }
        self.inner.append_applied(record).await
    }

    async fn write_snapshot(&self, snapshot: &ZoneSnapshot) -> anyhow::Result<Seq> {
        self.inner.write_snapshot(snapshot).await
    }

    async fn read_snapshot(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<Option<StoredSnapshot>> {
        self.inner.read_snapshot(zone, epoch).await
    }

    async fn write_watermark(&self, watermark: &Watermark) -> anyhow::Result<Seq> {
        self.inner.write_watermark(watermark).await
    }

    async fn epoch_status(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<EpochStatus> {
        self.inner.epoch_status(zone, epoch).await
    }

    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>> {
        self.inner.latest_epoch(zone).await
    }

    async fn read_epoch(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<RecordStream> {
        self.inner.read_epoch(zone, epoch).await
    }

    async fn append_session_in(&self, records: &[SessionInRecord]) -> anyhow::Result<u64> {
        if !records.is_empty() && self.hang_audit.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.down.load(Ordering::SeqCst) {
            anyhow::bail!("broker unavailable (injected)");
        }
        self.inner.append_session_in(records).await
    }

    async fn append_session_out(&self, records: &[SessionOutRecord]) -> anyhow::Result<u64> {
        if !records.is_empty() && self.hang_audit.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.down.load(Ordering::SeqCst) {
            anyhow::bail!("broker unavailable (injected)");
        }
        self.inner.append_session_out(records).await
    }
}
