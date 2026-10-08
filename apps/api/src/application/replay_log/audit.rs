//! `SessionAuditWriter`: the per-session `.in` / `.out` audit log (plan D5, §8 #3).
//!
//! Unlike the applied-tick log this is off the tick path and best effort: the session layer
//! (Epic 4) calls [`SessionAuditWriter::record_in`] / [`SessionAuditWriter::record_out`],
//! which never block. Frames go into a bounded buffer; a background task drains it in batches
//! and appends them (pipelined, acknowledged). A full buffer or an unacknowledged append drops
//! the frames and counts them in `eventlog_audit_dropped_total`, which must stay at zero
//! (alerted on). Replay never depends on these: the zone's applied log is the input.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::port::{EventLog, ReplayLogMetrics};
use super::record::{SessionInRecord, SessionOutRecord};

/// Default buffer: about two seconds of 200 sessions at 30 inbound + 10 outbound frames/s.
pub const AUDIT_BUFFER: usize = 16_384;

/// Most frames appended in one batch.
const BATCH: usize = 512;

enum AuditFrame {
    In(SessionInRecord),
    Out(SessionOutRecord),
}

/// Cheap, cloneable, non-blocking handle to the audit log.
#[derive(Clone)]
pub struct SessionAuditWriter {
    tx: mpsc::Sender<AuditFrame>,
    dropped: Arc<AtomicU64>,
    metrics: Arc<dyn ReplayLogMetrics>,
}

impl SessionAuditWriter {
    /// Starts the drain task with a buffer of `capacity` frames. The task flushes what is
    /// buffered and ends when `shutdown` is cancelled or every writer is dropped.
    #[must_use]
    pub fn spawn(
        log: Arc<dyn EventLog>,
        metrics: Arc<dyn ReplayLogMetrics>,
        capacity: usize,
        shutdown: CancellationToken,
    ) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let writer = Self {
            tx,
            dropped: dropped.clone(),
            metrics: metrics.clone(),
        };
        let task = tokio::spawn(drain(rx, log, metrics, dropped, shutdown));
        (writer, task)
    }

    /// Records one inbound frame. `false` if it was dropped.
    pub fn record_in(&self, record: SessionInRecord) -> bool {
        self.push(AuditFrame::In(record))
    }

    /// Records one outbound frame. `false` if it was dropped.
    pub fn record_out(&self, record: SessionOutRecord) -> bool {
        self.push(AuditFrame::Out(record))
    }

    /// Frames dropped since the writer started.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn push(&self, frame: AuditFrame) -> bool {
        // Full or closed: drop and count, never wait (the session's socket loop must not
        // block on the audit log).
        if self.tx.try_send(frame).is_ok() {
            true
        } else {
            count_dropped(&self.dropped, self.metrics.as_ref(), 1);
            false
        }
    }
}

fn count_dropped(dropped: &AtomicU64, metrics: &dyn ReplayLogMetrics, n: u64) {
    if n > 0 {
        dropped.fetch_add(n, Ordering::Relaxed);
        metrics.audit_dropped(n);
    }
}

async fn drain(
    mut rx: mpsc::Receiver<AuditFrame>,
    log: Arc<dyn EventLog>,
    metrics: Arc<dyn ReplayLogMetrics>,
    dropped: Arc<AtomicU64>,
    shutdown: CancellationToken,
) {
    let mut batch = Vec::with_capacity(BATCH);
    loop {
        let open = tokio::select! {
            biased;
            n = rx.recv_many(&mut batch, BATCH) => n > 0,
            () = shutdown.cancelled() => {
                // Flush whatever is already buffered, then stop.
                rx.close();
                while rx.recv_many(&mut batch, BATCH).await > 0 {
                    flush(&mut batch, log.as_ref(), metrics.as_ref(), &dropped).await;
                }
                false
            },
        };
        flush(&mut batch, log.as_ref(), metrics.as_ref(), &dropped).await;
        if !open {
            return;
        }
    }
}

async fn flush(
    batch: &mut Vec<AuditFrame>,
    log: &dyn EventLog,
    metrics: &dyn ReplayLogMetrics,
    dropped: &AtomicU64,
) {
    if batch.is_empty() {
        return;
    }
    let mut ins = Vec::new();
    let mut outs = Vec::new();
    for frame in batch.drain(..) {
        match frame {
            AuditFrame::In(r) => ins.push(r),
            AuditFrame::Out(r) => outs.push(r),
        }
    }
    let lost_in = match log.append_session_in(&ins).await {
        Ok(failed) => failed,
        Err(e) => {
            tracing::warn!(error = %e, frames = ins.len(), "audit append failed; frames dropped");
            ins.len() as u64
        },
    };
    let lost_out = match log.append_session_out(&outs).await {
        Ok(failed) => failed,
        Err(e) => {
            tracing::warn!(error = %e, frames = outs.len(), "audit append failed; frames dropped");
            outs.len() as u64
        },
    };
    count_dropped(dropped, metrics, lost_in.saturating_add(lost_out));
}
