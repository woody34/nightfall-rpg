//! Bridges the socket audit port to the existing bounded, acknowledged replay-log writer.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::application::replay_log::{
    EventLog, ReplayLogMetrics, SessionAuditWriter, SessionInRecord, SessionOutRecord,
};
use crate::application::{Clock, SessionAudit, SessionAuditContext};
use crate::domain::SessionId;

/// Maximum wait for buffered frames at shutdown if the broker stops acknowledging.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// Non-blocking session audit adapter. Only the game-frame port reaches this adapter;
/// HTTP upgrade headers/query parameters and gRPC authorization never do.
pub struct EventLogSessionAudit {
    writer: SessionAuditWriter,
    clock: Arc<dyn Clock>,
}

impl EventLogSessionAudit {
    /// Starts the bounded writer. The caller must create the streams before spawning this
    /// adapter and retain the drain until sessions have stopped producing frames.
    pub fn spawn(
        log: Arc<dyn EventLog>,
        clock: Arc<dyn Clock>,
        metrics: Arc<dyn ReplayLogMetrics>,
        capacity: usize,
    ) -> (Self, SessionAuditDrain) {
        let stop = CancellationToken::new();
        let (writer, task) = SessionAuditWriter::spawn(log, metrics, capacity, stop.clone());
        (Self { writer, clock }, SessionAuditDrain { stop, task })
    }

    /// Frames lost to a full/closed queue or failed acknowledgement since startup.
    pub fn dropped(&self) -> u64 {
        self.writer.dropped()
    }
}

impl SessionAudit for EventLogSessionAudit {
    fn record_in(&self, session: SessionId, seq: Option<u32>, frame: &Bytes) {
        // Legacy port callers have no zone position. Zero denotes unavailable metadata.
        self.record_in_context(session, seq, frame, SessionAuditContext::default());
    }

    fn record_out(&self, session: SessionId, frame: &Bytes) {
        self.record_out_context(session, frame, SessionAuditContext::default());
    }

    fn record_in_context(
        &self,
        session: SessionId,
        seq: Option<u32>,
        frame: &Bytes,
        context: SessionAuditContext,
    ) {
        self.writer.record_in(SessionInRecord {
            session: session.as_uuid(),
            seq: u64::from(seq.unwrap_or(0)),
            zone: context.zone,
            epoch: context.epoch,
            tick_seen: context.tick,
            recv_unix_ms: self.clock.now().timestamp_millis(),
            frame: frame.clone(),
        });
    }

    fn record_out_context(&self, session: SessionId, frame: &Bytes, context: SessionAuditContext) {
        self.writer.record_out(SessionOutRecord {
            session: session.as_uuid(),
            zone: context.zone,
            epoch: context.epoch,
            tick: context.tick,
            frame: frame.clone(),
        });
    }
}

/// Owns the audit worker's lifetime. Dropping it aborts the worker (including startup error
/// paths); normal shutdown instead flushes buffered frames with a bounded wait.
pub struct SessionAuditDrain {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl SessionAuditDrain {
    /// Stops accepting frames and waits up to five seconds for buffered acknowledged writes.
    pub async fn shutdown(mut self) {
        self.stop.cancel();
        match tokio::time::timeout(FLUSH_TIMEOUT, &mut self.task).await {
            Ok(Ok(())) => {},
            Ok(Err(e)) => tracing::warn!(error = %e, "session audit drain failed"),
            Err(_) => {
                self.task.abort();
                tracing::warn!("session audit flush timed out; remaining frames may be lost");
            },
        }
    }
}

impl Drop for SessionAuditDrain {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn shutdown_aborts_a_worker_that_never_finishes_within_the_flush_budget() {
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        let drain = SessionAuditDrain {
            stop: CancellationToken::new(),
            task,
        };
        let started = tokio::time::Instant::now();
        drain.shutdown().await;
        assert_eq!(started.elapsed(), FLUSH_TIMEOUT);
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
    }

    #[tokio::test]
    async fn dropping_the_drain_aborts_its_worker_on_startup_failure() {
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        drop(SessionAuditDrain {
            stop: CancellationToken::new(),
            task,
        });
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
    }
}
