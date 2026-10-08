use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use sea_orm::sea_query::{Expr, LockBehavior, LockType};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::OutboxPublisher;
use crate::infrastructure::postgres::entities::outbox;
use crate::infrastructure::telemetry::{Metrics, OutboxStats, OutboxStatsSource};

const BATCH_SIZE: u64 = 100;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Relay gauges, readable from any thread; they back `outbox_pending` and
/// `outbox_lag_seconds` (see [`OutboxStatsSource`]).
#[derive(Debug, Default)]
pub struct RelayStats {
    pending: AtomicU64,
    /// Creation time (unix microseconds) of the oldest unpublished row; 0 when none. Lag is
    /// derived from it when read, so it keeps growing even if the relay is stuck mid-poll.
    oldest_pending_micros: AtomicI64,
}

impl RelayStats {
    /// Rows still unpublished as of the last poll.
    #[must_use]
    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Relaxed)
    }

    /// Age in seconds of the oldest unpublished row; 0 when the outbox is empty.
    #[must_use]
    pub fn oldest_pending_age_seconds(&self) -> f64 {
        match self.oldest_pending_micros.load(Ordering::Relaxed) {
            0 => 0.0,
            created => {
                let age = Utc::now().timestamp_micros().saturating_sub(created).max(0);
                age as f64 / 1_000_000.0
            },
        }
    }
}

impl OutboxStatsSource for RelayStats {
    fn snapshot(&self) -> OutboxStats {
        OutboxStats {
            pending: self.pending(),
            lag_seconds: self.oldest_pending_age_seconds(),
        }
    }
}

/// Handle to the running relay task.
pub struct OutboxRelay {
    stats: Arc<RelayStats>,
    task: JoinHandle<()>,
}

impl OutboxRelay {
    /// Spawns the relay and registers its stats as the source of `metrics`' outbox gauges.
    /// It runs until `shutdown` is cancelled.
    #[must_use]
    pub fn spawn(
        db: DatabaseConnection,
        publisher: Arc<dyn OutboxPublisher>,
        metrics: &Metrics,
        shutdown: CancellationToken,
    ) -> Self {
        let stats = Arc::new(RelayStats::default());
        metrics.set_outbox_source(stats.clone());
        let task = tokio::spawn(run(db, publisher, shutdown, stats.clone()));
        Self { stats, task }
    }

    /// Live gauges.
    #[must_use]
    pub fn stats(&self) -> Arc<RelayStats> {
        self.stats.clone()
    }

    /// Waits for the task to finish (after cancelling the token).
    pub async fn join(self) {
        if let Err(e) = self.task.await {
            tracing::error!(error = %e, "outbox relay task failed");
        }
    }
}

async fn run(
    db: DatabaseConnection,
    publisher: Arc<dyn OutboxPublisher>,
    shutdown: CancellationToken,
    stats: Arc<RelayStats>,
) {
    let mut backoff = POLL_INTERVAL;
    loop {
        let wait = match relay_batch(&db, publisher.as_ref(), &stats).await {
            Ok(n) if n == BATCH_SIZE => Duration::ZERO, // more is waiting
            Ok(_) => {
                backoff = POLL_INTERVAL;
                POLL_INTERVAL
            },
            Err(e) => {
                tracing::warn!(error = %e, retry_in = ?backoff, "outbox relay batch failed");
                let w = backoff;
                backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                w
            },
        };
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(wait) => {},
        }
    }
}

/// One transaction: lock up to a batch of pending rows, publish them in id order, mark the
/// ones that were acknowledged. A publish failure stops the batch; rows already acknowledged
/// are still marked (committed), the rest stay pending. Returns the rows published.
async fn relay_batch(
    db: &DatabaseConnection,
    publisher: &dyn OutboxPublisher,
    stats: &RelayStats,
) -> anyhow::Result<u64> {
    let tx = db.begin().await?;
    let rows = outbox::Entity::find()
        .filter(outbox::Column::PublishedAt.is_null())
        .order_by_asc(outbox::Column::Id)
        .limit(BATCH_SIZE)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .all(&tx)
        .await?;

    let mut done = Vec::with_capacity(rows.len());
    let mut failure = None;
    for row in &rows {
        let payload = Bytes::from(serde_json::to_vec(&row.payload)?);
        match publisher.publish(&row.subject, row.id, payload).await {
            Ok(()) => {
                done.push(row.id);
            },
            Err(e) => {
                failure = Some(e.context(format!("publish outbox row {}", row.id)));
                break;
            },
        }
    }

    if !done.is_empty() {
        outbox::Entity::update_many()
            .col_expr(outbox::Column::PublishedAt, Expr::current_timestamp())
            .filter(outbox::Column::Id.is_in(done.clone()))
            .exec(&tx)
            .await?;
    }
    tx.commit().await?;

    // Refreshed on every poll, including ones that hit a publish failure, so a stalled relay
    // shows a growing lag and a drained outbox shows zero.
    let unpublished = outbox::Entity::find().filter(outbox::Column::PublishedAt.is_null());
    let oldest = unpublished
        .clone()
        .order_by_asc(outbox::Column::Id)
        .one(db)
        .await?;
    let pending = unpublished.count(db).await?;
    stats.pending.store(pending, Ordering::Relaxed);
    stats.oldest_pending_micros.store(
        oldest.map_or(0, |r| r.created_at.timestamp_micros().max(1)),
        Ordering::Relaxed,
    );

    match failure {
        Some(e) => Err(e),
        None => Ok(done.len() as u64),
    }
}
