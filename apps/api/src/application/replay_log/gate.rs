//! `DurableTickGate`: the zone actor's [`TickGate`] that appends each tick's record to the
//! replay log and waits for the acknowledgement before the tick is released (plan §8 #2).
//!
//! On failure it retries with exponential backoff while the actor stalls: no record, no
//! progress. Every failed attempt counts `eventlog_append_failures_total` and logs a warning.
//! After [`GateConfig::max_stall`] without an ack the zone is **paused**: `ZoneHandle::send`
//! refuses inputs (sessions answer `IntentRejected{OVERLOADED}`) and `zones_paused` goes up,
//! until an append succeeds again. Only shutdown makes `admit` give up.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::epoch::EpochStarted;
use super::port::{EventLog, ReplayLogMetrics};
use super::record::{AppliedTickRecord, Seq};
use crate::application::zone_actor::{GateError, TickGate};
use crate::domain::zone::{AppliedTick, Tick, ZoneId};

/// Retry and degradation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateConfig {
    /// First retry delay.
    pub initial_backoff: Duration,
    /// Retry delay cap.
    pub max_backoff: Duration,
    /// How long the zone may stall before it is paused.
    pub max_stall: Duration,
}

impl Default for GateConfig {
    /// 10 ms doubling to 1 s; paused after 5 s (50 missed ticks).
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_secs(1),
            max_stall: Duration::from_secs(5),
        }
    }
}

/// What has been durably logged in the epoch so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EpochProgress {
    /// The last tick whose record was acknowledged.
    pub last_tick: Option<Tick>,
    /// Log position of the epoch's first applied record.
    pub first_seq: Option<Seq>,
    /// Records acknowledged.
    pub records: u64,
}

/// The durable [`TickGate`]. Construct with [`DurableTickGate::new`] from the
/// [`EpochStarted`] that writing the snapshot returned.
pub struct DurableTickGate {
    zone: ZoneId,
    epoch: u64,
    log: Arc<dyn EventLog>,
    metrics: Arc<dyn ReplayLogMetrics>,
    config: GateConfig,
    progress: watch::Sender<EpochProgress>,
    next_tick: parking_lot::Mutex<Tick>,
    paused: watch::Sender<bool>,
    shutdown: CancellationToken,
}

impl DurableTickGate {
    /// A gate for the epoch whose snapshot `started` proves is logged. `shutdown` makes a
    /// stalled `admit` give up so the actor can stop.
    #[must_use]
    pub fn new(
        started: &EpochStarted,
        log: Arc<dyn EventLog>,
        metrics: Arc<dyn ReplayLogMetrics>,
        config: GateConfig,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            zone: started.zone(),
            epoch: started.epoch(),
            log,
            metrics,
            config,
            progress: watch::channel(EpochProgress::default()).0,
            next_tick: parking_lot::Mutex::new(started.first_tick()),
            paused: watch::channel(false).0,
            shutdown,
        }
    }

    /// Live view of what has been logged; the watermark is written from it.
    #[must_use]
    pub fn progress(&self) -> watch::Receiver<EpochProgress> {
        self.progress.subscribe()
    }

    fn set_paused(&self, paused: bool) {
        let changed = self.paused.send_if_modified(|p| {
            let changed = *p != paused;
            *p = paused;
            changed
        });
        if changed {
            self.metrics.zone_paused(paused);
            if paused {
                tracing::warn!(
                    zone = self.zone.0,
                    epoch = self.epoch,
                    max_stall = ?self.config.max_stall,
                    "replay log unavailable: zone paused, inputs refused as OVERLOADED"
                );
            } else {
                tracing::info!(
                    zone = self.zone.0,
                    epoch = self.epoch,
                    "replay log back: zone resumed"
                );
            }
        }
    }

    async fn append(&self, tick: &AppliedTick) -> Result<(), GateError> {
        let expected = *self.next_tick.lock();
        if tick.epoch != self.epoch || tick.tick != expected {
            // A programming error: the actor offers ticks in order. Refusing keeps the log
            // contiguous.
            return Err(GateError(format!(
                "record for epoch {} tick {} does not continue epoch {} at tick {}",
                tick.epoch, tick.tick.0, self.epoch, expected.0
            )));
        }
        let record = AppliedTickRecord::from_applied(self.zone, tick);
        let mut backoff = self.config.initial_backoff;
        let mut stalled_since: Option<tokio::time::Instant> = None;
        loop {
            match self.log.append_applied(&record).await {
                Ok(seq) => {
                    if stalled_since.is_some() {
                        tracing::info!(
                            zone = self.zone.0,
                            tick = tick.tick.0,
                            "replay log append recovered"
                        );
                    }
                    self.set_paused(false);
                    *self.next_tick.lock() = tick.tick.next();
                    self.progress.send_modify(|p| {
                        p.last_tick = Some(tick.tick);
                        p.first_seq.get_or_insert(seq);
                        p.records = p.records.saturating_add(1);
                    });
                    return Ok(());
                },
                Err(e) => {
                    self.metrics.append_failed();
                    let since = *stalled_since.get_or_insert_with(tokio::time::Instant::now);
                    tracing::warn!(
                        zone = self.zone.0,
                        tick = tick.tick.0,
                        error = %e,
                        retry_in = ?backoff,
                        "replay log append failed; zone stalled"
                    );
                    if since.elapsed() >= self.config.max_stall {
                        self.set_paused(true);
                    }
                    tokio::select! {
                        () = self.shutdown.cancelled() => {
                            return Err(GateError(format!("shut down while stalled: {e}")));
                        },
                        () = tokio::time::sleep(backoff) => {},
                    }
                    backoff = backoff.saturating_mul(2).min(self.config.max_backoff);
                },
            }
        }
    }
}

impl TickGate for DurableTickGate {
    fn admit(&self, tick: &AppliedTick) -> impl Future<Output = Result<(), GateError>> + Send {
        self.append(tick)
    }

    fn paused(&self) -> watch::Receiver<bool> {
        self.paused.subscribe()
    }
}
