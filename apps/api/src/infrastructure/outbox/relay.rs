use std::sync::atomic::{AtomicU64, Ordering};
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

const BATCH_SIZE: u64 = 100;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Relay gauges, readable from any thread. The telemetry epic maps these onto
/// `outbox_pending` and `outbox_lag_seconds`.
#[derive(Debug, Default)]
pub struct RelayStats {
    pending: AtomicU64,
    /// Lag of the most recent publish, in microseconds (age of the row when it was sent).
    last_lag_micros: AtomicU64,
}

impl RelayStats {
    /// Rows still unpublished as of the last poll.
    #[must_use]
    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Relaxed)
    }

    /// Seconds between a row being staged and the relay publishing it, for the last row sent.
    #[must_use]
    pub fn last_publish_lag_seconds(&self) -> f64 {
        self.last_lag_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }
}

/// Handle to the running relay task.
pub struct OutboxRelay {
    stats: Arc<RelayStats>,
    task: JoinHandle<()>,
}

impl OutboxRelay {
    /// Spawns the relay. It runs until `shutdown` is cancelled.
    #[must_use]
    pub fn spawn(
        db: DatabaseConnection,
        publisher: Arc<dyn OutboxPublisher>,
        shutdown: CancellationToken,
    ) -> Self {
        let stats = Arc::new(RelayStats::default());
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
                let lag = Utc::now().signed_duration_since(row.created_at);
                let micros = u64::try_from(lag.num_microseconds().unwrap_or(0)).unwrap_or(0);
                stats.last_lag_micros.store(micros, Ordering::Relaxed);
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

    let pending = outbox::Entity::find()
        .filter(outbox::Column::PublishedAt.is_null())
        .count(db)
        .await?;
    stats.pending.store(pending, Ordering::Relaxed);

    match failure {
        Some(e) => Err(e),
        None => Ok(done.len() as u64),
    }
}
